//! Remote desktop and screen sharing, and the ssh tunnels that carry them through a
//! jump chain. RDP is either handed to the system client as a `.rdp` file or decoded
//! in-app by `rdp_session.rs`; VNC is always a handoff to `vnc://`.

use super::{blocking, load_jacks, os_open};
use crate::{patchbay, rdp, rdp_session};
use serde::Serialize;

#[derive(Serialize)]
pub struct TunnelView {
    id: u32,
    jack: String,
    local: u16,
    via: String,
}

/// Ids from a counter, not a clock: a collision would overwrite the map entry and
/// leak the ssh child with nothing left to kill it.
fn next_tunnel_id() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Where to dial a device's port from here: the host itself, or a local port
/// forwarded over the jump chain when it sits behind one.
fn dial_address(
    shared: &rdp::SharedTunnels,
    jacks: &patchbay::Jacks,
    resolved: &str,
    port: u16,
) -> Result<String, String> {
    let j = jacks
        .get(resolved)
        .ok_or_else(|| format!("no jack named \"{resolved}\""))?;
    let hops = patchbay::hops(resolved, jacks)?;

    Ok(if hops.is_empty() {
        format!("{}:{port}", j.host)
    } else {
        let local = rdp::free_port()?;
        // -N: no shell, just the forward. The chain's far end is the box we tunnel from.
        let mut args = vec![
            "-N".to_string(),
            "-L".to_string(),
            format!("127.0.0.1:{local}:{}:{port}", j.host),
            "-J".to_string(),
            hops.join(","),
        ];
        args.push(hops.last().cloned().unwrap_or_default());
        shared.open(next_tunnel_id(), resolved, &args, local, hops.join(" → "))?;
        format!("127.0.0.1:{local}")
    })
}

/// Address, resolved name, configured user and certificate pin for a device's RDP port.
fn rdp_address(
    shared: &rdp::SharedTunnels,
    name: &str,
) -> Result<(String, String, Option<String>, String), String> {
    let jacks = load_jacks()?;
    let (resolved, port) = rdp_port(&jacks, name)?;
    let pin = rdp_pin(&jacks, name)?;
    let addr = dial_address(shared, &jacks, &resolved, port)?;
    Ok((addr, resolved.clone(), jacks[&resolved].user.clone(), pin))
}

fn rdp_port(jacks: &patchbay::Jacks, name: &str) -> Result<(String, u16), String> {
    let resolved = patchbay::resolve(name, jacks)?;
    let port = jacks
        .get(&resolved)
        .ok_or_else(|| format!("no jack named \"{resolved}\""))?
        .rdp
        .ok_or_else(|| format!("\"{resolved}\" has no rdp port"))?;
    Ok((resolved, port))
}

/// What a device's RDP certificate is remembered under: its own address, never the
/// dialled one, which is `127.0.0.1` for everything behind a bastion.
fn rdp_pin(jacks: &patchbay::Jacks, name: &str) -> Result<String, String> {
    let (resolved, port) = rdp_port(jacks, name)?;
    Ok(format!("{}:{port}", jacks[&resolved].host))
}

/// Remote desktop in the system client, via a `.rdp` file.
#[tauri::command]
pub async fn open_rdp(
    tunnels: tauri::State<'_, rdp::SharedTunnels>,
    name: String,
) -> Result<String, String> {
    let shared = tunnels.inner().clone();
    blocking(move || {
        let (addr, resolved, user, _) = rdp_address(&shared, &name)?;
        let body = rdp::rdp_file(&addr, user.as_deref())?;
        let path = rdp::write_file(&resolved, &body)?;
        rdp::hand_off(&path)?;
        Ok(addr)
    })
    .await
}

/// `DOMAIN\user` split into the two fields RDP wants. A UPN (`user@domain`) is
/// already one field and passes through; `.\user` means a local account.
fn split_domain(user: &str) -> (Option<String>, String) {
    let Some((domain, name)) = user.split_once('\\') else {
        return (None, user.to_string());
    };
    (
        (!domain.is_empty() && domain != ".").then(|| domain.to_string()),
        name.to_string(),
    )
}

/// Remote desktop in a tab. Connects synchronously so a wrong password is an error the
/// sheet can show, then streams tiles through `on_tile`.
#[tauri::command]
pub async fn open_rdp_session(
    app: tauri::AppHandle,
    tunnels: tauri::State<'_, rdp::SharedTunnels>,
    sessions: tauri::State<'_, rdp_session::Shared>,
    id: u32,
    name: String,
    user: String,
    password: String,
    width: u16,
    height: u16,
    scale: u32,
    on_tile: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
) -> Result<rdp_session::Screen, String> {
    let shared = tunnels.inner().clone();
    let rdp_sessions = sessions.inner().clone();
    blocking(move || {
        let (addr, resolved, cfg_user, pin) = rdp_address(&shared, &name)?;
        let (host, port) = addr
            .rsplit_once(':')
            .ok_or_else(|| format!("\"{addr}\" isn't a host and port"))?;
        let port: u16 = port
            .parse()
            .map_err(|_| format!("\"{addr}\" has no usable port"))?;
        // The config's user only prefills the sign-in field.
        let user = Some(user)
            .filter(|u| !u.is_empty())
            .or(cfg_user)
            .ok_or_else(|| format!("\"{resolved}\" needs a user to sign in with"))?;
        let (domain, user) = split_domain(&user);
        rdp_sessions.open(
            id, host, port, &pin, user, password, domain, width, height, scale, on_tile, app,
        )
    })
    .await
}

/// A device's changed RDP certificate, trusted from here rather than by editing
/// `rdp_known_hosts`. Takes the fingerprint the refusal showed, so a certificate that
/// changed again in between is refused again rather than trusted unseen.
#[tauri::command]
pub async fn rdp_trust(name: String, fingerprint: String) -> Result<(), String> {
    blocking(move || rdp_session::trust::accept(&rdp_pin(&load_jacks()?, &name)?, &fingerprint))
        .await
}

#[tauri::command]
pub fn close_rdp_session(sessions: tauri::State<'_, rdp_session::Shared>, id: u32) {
    sessions.close(id);
}

/// Mouse and keyboard from the canvas. Fire-and-forget: input after the session ended
/// is not worth a dialog.
#[tauri::command]
pub fn rdp_input(
    sessions: tauri::State<'_, rdp_session::Shared>,
    id: u32,
    kind: String,
    a: i32,
    b: i32,
    down: bool,
    scale: Option<u32>,
) {
    let input = match kind.as_str() {
        "move" => rdp_session::Input::Move {
            x: a.clamp(0, 65535) as u16,
            y: b.clamp(0, 65535) as u16,
        },
        "button" => rdp_session::Input::Button {
            button: a.clamp(0, 255) as u8,
            down,
        },
        "wheel" => rdp_session::Input::Wheel {
            delta: a.clamp(-32768, 32767) as i16,
        },
        "key" => rdp_session::Input::Key {
            scancode: a.clamp(0, 65535) as u16,
            down,
        },
        // Not input, but it rides the same queue: the session thread is the only place
        // that may touch the socket.
        "resize" => rdp_session::Input::Resize {
            width: a.clamp(0, 65535) as u16,
            height: b.clamp(0, 65535) as u16,
            scale: scale.unwrap_or(100),
        },
        _ => return,
    };
    sessions.send(id, input);
}

/// Files dropped on a remote desktop tab, put where the far end sees them as a drive.
#[tauri::command]
pub async fn rdp_drop(paths: Vec<String>) -> Result<String, String> {
    if cfg!(windows) {
        return Err("sharing files into a remote desktop needs macOS or Linux".into());
    }
    let n = blocking(move || crate::drive::copy_in(&paths)).await?;
    let files = if n == 1 { "file" } else { "files" };
    Ok(format!(
        "{n} {files} in \\\\tsclient\\{}",
        crate::drive::DRIVE
    ))
}

/// Screen sharing, handed to whatever viewer owns `vnc://` on this machine.
#[tauri::command]
pub async fn open_vnc(
    tunnels: tauri::State<'_, rdp::SharedTunnels>,
    name: String,
) -> Result<String, String> {
    let shared = tunnels.inner().clone();
    blocking(move || {
        let jacks = load_jacks()?;
        let resolved = patchbay::resolve(&name, &jacks)?;
        let j = jacks
            .get(&resolved)
            .ok_or_else(|| format!("no jack named \"{resolved}\""))?;
        let port = j
            .vnc
            .ok_or_else(|| format!("\"{resolved}\" has no vnc port"))?;
        let url = vnc_url(
            &dial_address(&shared, &jacks, &resolved, port)?,
            j.user.as_deref(),
        )?;
        os_open(std::ffi::OsStr::new(&url))?;
        Ok(url)
    })
    .await
}

/// This reaches the desktop opener, so an `@` or `/` in the username would move the
/// host the viewer dials. A name that can't be carried is dropped; the viewer asks.
fn vnc_url(addr: &str, user: Option<&str>) -> Result<String, String> {
    if !rdp::is_safe(addr) || addr.contains(['/', '@', '?', '#', ' ']) {
        return Err(format!("\"{addr}\" isn't a usable address"));
    }
    let ok = |u: &&str| {
        u.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    };
    Ok(
        match user.map(str::trim).filter(|u| !u.is_empty()).filter(ok) {
            Some(u) => format!("vnc://{u}@{addr}"),
            None => format!("vnc://{addr}"),
        },
    )
}

/// The device's own `forward` list, held open on its own so a database port doesn't
/// go down with the terminal tab that happened to be carrying it.
#[tauri::command]
pub async fn open_forwards(
    tunnels: tauri::State<'_, rdp::SharedTunnels>,
    name: String,
) -> Result<u16, String> {
    let shared = tunnels.inner().clone();
    blocking(move || {
        let jacks = load_jacks()?;
        let resolved = patchbay::resolve(&name, &jacks)?;
        let j = jacks
            .get(&resolved)
            .ok_or_else(|| format!("no jack named \"{resolved}\""))?;
        let forwards = j.forward.as_deref().unwrap_or_default();
        if forwards.is_empty() {
            return Err(format!("\"{resolved}\" has no forward to open"));
        }
        let ports: Vec<u16> = forwards
            .iter()
            .filter_map(|f| patchbay::forward_local(f))
            .collect();
        // Check every port up front: ExitOnForwardFailure ends ssh if any one can't
        // bind, which would otherwise surface as a slow "never came up".
        if let Some(p) = ports.iter().copied().find(|p| rdp::port_taken(*p)) {
            return Err(format!("something is already listening on 127.0.0.1:{p}"));
        }
        // All-remote forwards have no local port to watch; 0 means "judge by ssh
        // staying alive".
        let local = ports.first().copied().unwrap_or(0);
        let mut args = vec![
            "-N".to_string(),
            "-o".to_string(),
            "ExitOnForwardFailure=yes".to_string(),
        ];
        args.extend(patchbay::ssh_args(&resolved, &jacks)?);
        let hops = patchbay::hops(&resolved, &jacks)?;
        let via = if hops.is_empty() {
            patchbay::spec(j)
        } else {
            hops.join(" → ")
        };
        shared.open(next_tunnel_id(), &resolved, &args, local, via)?;
        Ok(local)
    })
    .await
}

/// The live tunnels, plus a line for each one that died since the last ask, so the
/// window can say so rather than show a port that stopped answering.
#[derive(Serialize)]
pub struct TunnelList {
    live: Vec<TunnelView>,
    ended: Vec<String>,
}

#[tauri::command]
pub fn tunnels(state: tauri::State<'_, rdp::SharedTunnels>) -> TunnelList {
    let (live, ended) = state.list();
    TunnelList {
        live: live
            .into_iter()
            .map(|(id, jack, local, via)| TunnelView {
                id,
                jack,
                local,
                via,
            })
            .collect(),
        ended: ended
            .into_iter()
            .map(|(jack, why)| {
                if why.is_empty() {
                    format!("the tunnel to {jack} ended")
                } else {
                    format!("the tunnel to {jack} ended: {why}")
                }
            })
            .collect(),
    }
}

#[tauri::command]
pub fn close_tunnel(state: tauri::State<'_, rdp::SharedTunnels>, id: u32) {
    state.close(id);
}

/// Wake on LAN: the magic packet, broadcast on this network. There is nothing to
/// connect to yet, so nothing comes back and the window only learns that it was sent.
///
/// ponytail: the broadcast address, port 9, from whatever interface the route picks.
/// A device on another VLAN needs its subnet's directed broadcast and a router that
/// forwards it, which is a `wake_via` setting the day somebody has one - and a device
/// behind a jump chain wants the packet sent from the bastion, which is another day
/// again.
#[tauri::command]
pub async fn wake(name: String) -> Result<String, String> {
    blocking(move || {
        let jacks = load_jacks()?;
        let resolved = patchbay::resolve(&name, &jacks)?;
        let mac = jacks
            .get(&resolved)
            .and_then(|j| j.mac.as_deref())
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .ok_or_else(|| format!("\"{resolved}\" has no mac address to wake"))?;
        let packet = patchbay::magic_packet(mac)?;

        let socket = std::net::UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("no socket to send from: {e}"))?;
        socket
            .set_broadcast(true)
            .map_err(|e| format!("this machine won't broadcast: {e}"))?;
        socket
            .send_to(&packet, "255.255.255.255:9")
            .map_err(|e| format!("\"{resolved}\": {e}"))?;
        Ok(format!("woke {resolved} at {mac}"))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{split_domain, vnc_url};

    #[test]
    fn a_domain_login_splits_into_the_two_fields_rdp_wants() {
        assert_eq!(
            split_domain("CORP\\alice"),
            (Some("CORP".into()), "alice".into())
        );
        assert_eq!(split_domain("alice"), (None, "alice".into()));
        assert_eq!(
            split_domain("alice@corp.example"),
            (None, "alice@corp.example".into())
        );
        // Entra-joined boxes want the UPN kept whole behind the AzureAD prefix.
        assert_eq!(
            split_domain("AzureAD\\alice@corp.example"),
            (Some("AzureAD".into()), "alice@corp.example".into())
        );
        assert_eq!(
            split_domain(".\\alice"),
            (None, "alice".into()),
            "a local account"
        );
        assert_eq!(split_domain("\\alice"), (None, "alice".into()));
    }

    #[test]
    fn a_vnc_url_carries_only_a_name_it_can_carry() {
        assert_eq!(
            vnc_url("10.0.0.9:5900", Some("alice")).unwrap(),
            "vnc://alice@10.0.0.9:5900"
        );
        assert_eq!(
            vnc_url("10.0.0.9:5900", Some("  ")).unwrap(),
            "vnc://10.0.0.9:5900"
        );
        // Dropped, not refused: the viewer asks for the name itself.
        assert_eq!(
            vnc_url("127.0.0.1:5901", Some("me@evil.example")).unwrap(),
            "vnc://127.0.0.1:5901"
        );
        assert!(vnc_url("10.0.0.9:5900/../x", None).is_err());
        assert!(vnc_url("bad host\u{7}", None).is_err());
    }
}
