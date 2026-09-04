// CRT plasma behind the hero: React Bits' CRTWarp shader on raw WebGL, one fullscreen quad,
// because this page has no bundler. Colours come from the theme tokens.
(function () {
  var hero = document.querySelector(".hero");
  var canvas = document.getElementById("crt");
  if (!hero || !canvas) return;

  var slow = window.matchMedia("(prefers-reduced-motion: reduce)");
  var gl = canvas.getContext("webgl", {
    antialias: false,
    alpha: false,
    powerPreference: "low-power",
  });
  if (!gl) {
    canvas.remove();
    return;
  }

  var VERT =
    "attribute vec2 p;varying vec2 vUv;void main(){vUv=p*0.5+0.5;gl_Position=vec4(p,0.0,1.0);}";

  var FRAG = [
    "precision highp float;",
    "varying vec2 vUv;",
    "uniform vec2 uResolution;",
    "uniform float uTime;",
    "uniform vec3 uColor;",
    "uniform vec3 uBackgroundColor;",
    "uniform float uCurvature, uScanlineStrength, uScanlineFrequency;",
    "uniform float uWaveAmplitude, uWaveFrequency, uBloom, uBloomRadius;",
    "uniform float uNoise, uVignette, uBrightness, uRgbShift, uMouseStrength;",
    "uniform vec2 uPointer;",

    "float hash21(vec2 p){p=fract(p*vec2(123.34,456.21));p+=dot(p,p+45.32);return fract(p.x*p.y);}",

    "vec2 crtCurve(vec2 uv, float radius){",
    "  vec2 p=(uv-0.5)*2.0;",
    "  float r=max(radius,1.415);",
    "  float cornerScale=r/sqrt(max(r*r-2.0,0.001));",
    "  p=r*p/sqrt(max(r*r-dot(p,p),0.001));",
    "  p/=cornerScale;",
    "  return p*0.5+0.5;",
    "}",

    "float plasma(vec2 uv, float t){",
    "  float f=max(uWaveFrequency/2.2,0.001);",
    "  uv=(uv-0.5)*f+0.5;",
    "  float scan=0.5-0.5*cos(uv.y*3.14159265*uScanlineFrequency);",
    "  scan=mix(1.0,scan,uScanlineStrength);",
    "  uv*=vec2(80.0,24.0); uv=ceil(uv); uv/=vec2(80.0,24.0);",
    "  float amp=uWaveAmplitude/0.28;",
    "  float v=0.0;",
    "  v+=0.7*sin(0.5*uv.x+t/5.0);",
    "  v+=3.0*sin(1.6*uv.y+t/5.0);",
    "  v+=sin(10.0*(uv.y*sin(t/2.0)+uv.x*cos(t/5.0))+t/2.0);",
    "  float cx=uv.x+0.5*sin(t/2.0);",
    "  float cy=uv.y+0.5*cos(t/4.0);",
    "  v+=0.4*sin(sqrt(100.0*cx*cx+100.0*cy*cy+1.0)+t);",
    "  v+=0.9*sin(sqrt(75.0*cx*cx+25.0*cy*cy+1.0)+t);",
    "  v-=1.4*sin(sqrt(256.0*cx*cx+25.0*cy*cy+1.0)+t);",
    "  v+=0.3*sin(0.5*uv.y+uv.x+sin(t));",
    "  return scan*floor(3.0*(0.5+0.499*sin(v*amp)))/3.0;",
    "}",

    "void main(){",
    "  float curveRadius=(1.1+0.42/max(uCurvature,0.001))*exp(-uPointer.y*uMouseStrength*0.4);",
    "  vec2 uv=crtCurve(vUv,curveRadius);",
    "  uv.x-=uPointer.x*uMouseStrength*0.035;",
    "  float s=plasma(uv,uTime);",
    "  float r=0.01*uBloomRadius;",
    "  float glow=s*0.2;",
    "  glow+=plasma(uv+vec2(r,0.0),uTime)*0.12;",
    "  glow+=plasma(uv-vec2(r,0.0),uTime)*0.12;",
    "  glow+=plasma(uv+vec2(0.0,r),uTime)*0.12;",
    "  glow+=plasma(uv-vec2(0.0,r),uTime)*0.12;",
    "  glow+=plasma(uv+vec2(r),uTime)*0.08;",
    "  glow+=plasma(uv-vec2(r),uTime)*0.08;",
    "  glow+=plasma(uv+vec2(r,-r),uTime)*0.08;",
    "  glow+=plasma(uv+vec2(-r,r),uTime)*0.08;",
    "  float red=plasma(uv+vec2(uRgbShift,0.0),uTime);",
    "  float blue=plasma(uv-vec2(uRgbShift,0.0),uTime);",
    "  vec3 wave=uColor*(0.3+s*0.7+glow*uBloom*0.65);",
    "  wave+=(vec3(red,s,blue)-s)*0.42;",
    "  float edge=clamp(1.0-dot(vUv-0.5,vUv-0.5)*2.0,0.0,1.0);",
    "  float fade=mix(1.0,smoothstep(0.0,1.0,edge),uVignette);",
    "  float mask=clamp(s*0.82+glow*0.52,0.0,1.0)*fade;",
    "  float grain=hash21(gl_FragCoord.xy+vec2(fract(uTime)*173.0));",
    "  wave=max(wave*uBrightness,vec3(0.0));",
    "  vec3 c=mix(uBackgroundColor,wave,mask);",
    "  c+=(grain-0.5)*uNoise;",
    "  gl_FragColor=vec4(clamp(c,0.0,1.0),1.0);",
    "}",
  ].join("\n");

  function compile(type, src) {
    var sh = gl.createShader(type);
    gl.shaderSource(sh, src);
    gl.compileShader(sh);
    if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
      gl.deleteShader(sh);
      return null;
    }
    return sh;
  }
  var vs = compile(gl.VERTEX_SHADER, VERT),
    fs = compile(gl.FRAGMENT_SHADER, FRAG);
  if (!vs || !fs) {
    canvas.remove();
    return;
  }
  var prog = gl.createProgram();
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    canvas.remove();
    return;
  }
  gl.useProgram(prog);

  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  var loc = gl.getAttribLocation(prog, "p");
  gl.enableVertexAttribArray(loc);
  gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

  var U = {};
  [
    "uResolution",
    "uTime",
    "uColor",
    "uBackgroundColor",
    "uCurvature",
    "uScanlineStrength",
    "uScanlineFrequency",
    "uWaveAmplitude",
    "uWaveFrequency",
    "uBloom",
    "uBloomRadius",
    "uNoise",
    "uVignette",
    "uBrightness",
    "uRgbShift",
    "uPointer",
    "uMouseStrength",
  ].forEach(function (n) {
    U[n] = gl.getUniformLocation(prog, n);
  });

  // Dimmer and slower than the component's demo: it sits behind a headline.
  gl.uniform1f(U.uCurvature, 0.22);
  gl.uniform1f(U.uScanlineStrength, 0.18);
  gl.uniform1f(U.uScanlineFrequency, 260.0);
  gl.uniform1f(U.uWaveAmplitude, 0.28);
  gl.uniform1f(U.uWaveFrequency, 2.2);
  gl.uniform1f(U.uBloom, 1.2);
  gl.uniform1f(U.uBloomRadius, 1.0);
  gl.uniform1f(U.uNoise, 0.025);
  gl.uniform1f(U.uVignette, 0.45);
  gl.uniform1f(U.uRgbShift, 0.012);
  gl.uniform1f(U.uMouseStrength, 0.45);

  var css = getComputedStyle(document.documentElement);
  // Straight to display space, so the dark cells match --ground and the canvas has no edge.
  function put(u, name, fallback) {
    var v = (css.getPropertyValue(name) || "").trim() || fallback;
    var m = /^#?([0-9a-f]{6})$/i.exec(v);
    var n = m ? parseInt(m[1], 16) : parseInt(fallback.slice(1), 16);
    gl.uniform3f(u, ((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255);
  }
  var light = false;
  function theme() {
    put(U.uColor, "--accent", "#6aa6ff");
    put(U.uBackgroundColor, "--ground", "#0c0d11");
    light =
      getComputedStyle(hero).getPropertyValue("color-scheme").indexOf("light") > -1 ||
      /^#?f/i.test((css.getPropertyValue("--ground") || "").trim());
    /* On a light ground the phosphor has to be quiet or it looks like a fault. */
    gl.uniform1f(U.uBrightness, light ? 0.5 : 0.8);
    canvas.style.opacity = light ? "0.5" : "0.72";
  }
  theme();
  new MutationObserver(theme).observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });
  if (window.matchMedia) {
    var mq = window.matchMedia("(prefers-color-scheme: light)");
    (mq.addEventListener ? mq.addEventListener.bind(mq, "change") : mq.addListener.bind(mq))(theme);
  }

  var dpr = Math.min(window.devicePixelRatio || 1, 1);
  function resize() {
    var w = Math.max(hero.clientWidth, 1),
      h = Math.max(hero.clientHeight, 1);
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.uniform2f(U.uResolution, canvas.width, canvas.height);
  }
  new ResizeObserver(resize).observe(hero);
  resize();

  var px = 0,
    py = 0,
    tx = 0,
    ty = 0;
  hero.addEventListener(
    "pointermove",
    function (e) {
      var r = hero.getBoundingClientRect();
      tx = ((e.clientX - r.left) / Math.max(r.width, 1)) * 2 - 1;
      ty = -(((e.clientY - r.top) / Math.max(r.height, 1)) * 2 - 1);
    },
    { passive: true },
  );
  hero.addEventListener("pointerleave", function () {
    tx = 0;
    ty = 0;
  });

  function draw(t) {
    gl.uniform1f(U.uTime, t);
    gl.uniform2f(U.uPointer, px, py);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  /* Reduced motion gets one still frame: the effect is there, it just holds. */
  if (slow.matches) {
    draw(8.0);
    return;
  }

  var seen = true;
  new IntersectionObserver(function (e) {
    seen = e[0].isIntersecting;
  }).observe(hero);

  var time = 0,
    last = 0,
    prev = 0,
    INTERVAL = 1000 / 30;
  (function loop(now) {
    requestAnimationFrame(loop);
    if (!seen || document.hidden) {
      prev = now;
      return;
    }
    if (now - last < INTERVAL) return;
    last = now - ((now - last) % INTERVAL);
    time += Math.min((now - prev) / 1000, 0.1) * 0.35;
    prev = now;
    px += (tx - px) * 0.08;
    py += (ty - py) * 0.08;
    draw(time);
  })(0);
})();

// The window tips towards the pointer, so it reads as a solid object catching the light.
(function () {
  var shot = document.querySelector(".shot");
  var win = shot && shot.querySelector(".win");
  if (!win) return;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  var raf = 0,
    tx = 0,
    ty = 0;
  shot.addEventListener(
    "pointermove",
    function (e) {
      var r = win.getBoundingClientRect();
      // -0.5 at one edge, +0.5 at the other
      tx = (e.clientX - r.left) / Math.max(r.width, 1) - 0.5;
      ty = (e.clientY - r.top) / Math.max(r.height, 1) - 0.5;
      if (!raf) raf = requestAnimationFrame(apply);
    },
    { passive: true },
  );

  function apply() {
    raf = 0;
    win.style.transform =
      "perspective(1600px) rotateX(" +
      (-ty * 4.5).toFixed(2) +
      "deg) rotateY(" +
      (tx * 6).toFixed(2) +
      "deg) scale(1.012)";
  }
  shot.addEventListener("pointerleave", function () {
    if (raf) {
      cancelAnimationFrame(raf);
      raf = 0;
    }
    win.style.transform = "";
  });
})();

// Click a row and the detail pane follows. The one interaction in the mock window that is wired up.
(function () {
  var list = document.querySelector(".w-list");
  var dname = document.getElementById("w-dname");
  var ddesc = document.getElementById("w-ddesc");
  var target = document.getElementById("w-target");
  var route = document.getElementById("w-route");
  var fwrap = document.getElementById("w-fwrap");
  var reach = document.getElementById("w-reach");
  var cmdtext = document.getElementById("w-cmdtext");
  if (!list || !dname) return;

  function row(host, user, ...more) {
    var rows = [["host", host]];
    if (user) rows.push(["user", user]);
    rows.push(...more);
    return rows
      .map(function (r) {
        return '<div class="w-row"><dt>' + r[0] + "</dt><dd>" + r[1] + "</dd></div>";
      })
      .join("");
  }
  function hops(chain, dest) {
    var spans = ['<span><i class="pip"></i>this machine</span>'];
    chain.forEach(function (h) {
      spans.push('<span><i class="pip"></i>' + h + '<i class="arm">jump</i></span>');
    });
    spans.push('<span class="last"><i class="pip"></i>' + dest + "</span>");
    return spans.join("");
  }

  var DATA = {
    bastion: {
      desc: "the way into infra",
      target: row("bastion.example", "root", ["port", "2222"]),
      route: hops([], "root@bastion.example:2222"),
      fwd: null,
      reach: { ok: true, via: "this machine", ms: 9 },
      cmd: "ssh -p 2222 root@bastion.example",
    },
    web: {
      desc: "webserver, eu",
      target: row("10.0.0.4", "deploy"),
      route: hops(["bastion.example:2222"], "deploy@10.0.0.4"),
      fwd: null,
      reach: { ok: true, via: "bastion.example:2222", ms: 19 },
      cmd: "ssh -J bastion.example:2222 deploy@10.0.0.4",
    },
    db: {
      desc: "primary, replicas behind it",
      target: row("10.0.0.5", "root"),
      route: hops(["bastion.example:2222", "deploy@10.0.0.4"], "root@10.0.0.5"),
      fwd: [["-L", "5432:localhost:5432"]],
      reach: { ok: true, via: "bastion.example:2222", ms: 24 },
      cmd: "ssh -J bastion.example:2222,deploy@10.0.0.4 -L 5432:localhost:5432 root@10.0.0.5",
    },
    "web-us": {
      desc: "webserver, us",
      target: row("10.1.0.4", "deploy"),
      route: hops([], "deploy@10.1.0.4"),
      fwd: null,
      reach: { ok: true, via: "this machine", ms: 61 },
      cmd: "ssh deploy@10.1.0.4",
    },
    "db-us": {
      desc: "replica, us",
      target: row("10.1.0.5", "root"),
      route: hops([], "root@10.1.0.5"),
      fwd: null,
      reach: { down: true },
      cmd: "ssh root@10.1.0.5",
    },
    "staging-web": {
      desc: "staging",
      target: row("10.2.0.4", "deploy"),
      route: hops([], "deploy@10.2.0.4"),
      fwd: null,
      reach: { unknown: true },
      cmd: "ssh deploy@10.2.0.4",
    },
    nas: {
      desc: "Synology, backups land here",
      target: row("10.0.0.20", "root"),
      route: hops([], "root@10.0.0.20"),
      fwd: null,
      reach: { down: true },
      cmd: "ssh root@10.0.0.20",
    },
    hypervisor: {
      desc: "Proxmox host",
      target: row("10.0.0.30", "root"),
      route: hops([], "root@10.0.0.30"),
      fwd: null,
      reach: { ok: true, via: "this machine", ms: 4 },
      cmd: "ssh root@10.0.0.30",
    },
    gateway: {
      desc: "OPNsense, the edge",
      target: row("10.0.0.1", "root"),
      route: hops([], "root@10.0.0.1"),
      fwd: null,
      reach: { ok: true, via: "this machine", ms: 2 },
      cmd: "ssh root@10.0.0.1",
    },
    pi: {
      desc: "homelab",
      target: row("10.0.0.40", "root"),
      route: hops([], "root@10.0.0.40"),
      fwd: null,
      reach: { down: true },
      cmd: "ssh root@10.0.0.40",
    },
    "acme-app": {
      desc: "acme's app server",
      target: row("10.0.0.10", "deploy"),
      route: hops([], "deploy@10.0.0.10"),
      fwd: null,
      reach: { ok: true, via: "this machine", ms: 71 },
      cmd: "ssh deploy@10.0.0.10",
    },
    "acme-dc": {
      desc: "acme's domain controller",
      target: row("10.0.0.50", "administrator"),
      route: hops([], "administrator@10.0.0.50"),
      fwd: null,
      reach: { ok: true, via: "this machine", ms: 68 },
      cmd: "ssh administrator@10.0.0.50",
    },
  };

  function reachHtml(r) {
    if (r.unknown) return '<span style="color:var(--ink-faint)">not probed yet</span>';
    if (r.down) return '<span style="color:var(--down)">down</span> &middot; timed out';
    return '<span style="color:var(--up)">up</span> &middot; ' + r.via + " &middot; " + r.ms + "ms";
  }

  // Marks and the dock they raise, the same gesture as the app: cmd-click picks a row out,
  // shift-click takes the run between, a plain click drops the lot.
  var dock = document.getElementById("w-dock");
  var ICON = {
    bcast:
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4.9 19.1C1 15.2 1 8.8 4.9 4.9"/><path d="M7.8 16.2c-2.3-2.3-2.3-6.1 0-8.5"/><circle cx="12" cy="12" r="2"/><path d="M16.2 7.8c2.3 2.3 2.3 6.1 0 8.5"/><path d="M19.1 4.9C23 8.8 23 15.1 19.1 19"/></svg>',
    del: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>',
  };

  function rows() {
    return [].slice.call(list.querySelectorAll(".w-jack"));
  }
  function paintDock() {
    var marks = rows().filter(function (r) {
      return r.hasAttribute("data-mark");
    });
    if (marks.length < 2) {
      dock.hidden = true;
      return;
    }
    // Only the ones patchbay would actually open a shell on, the way the app counts.
    var ssh = marks.filter(function (r) {
      return !r.querySelector(".w-web");
    }).length;
    dock.innerHTML =
      '<span class="count"><b>' +
      marks.length +
      "</b> selected</span>" +
      (ssh > 1 ? '<span class="a">' + ICON.bcast + "Broadcast to " + ssh + "</span>" : "") +
      '<span class="a danger">' +
      ICON.del +
      "Delete " +
      marks.length +
      "</span>";
    dock.hidden = false;
  }

  list.addEventListener("click", function (e) {
    var jack = e.target.closest(".w-jack");
    // Clicking the space under the rows is clicking away: the marks go, the selection stays.
    if (!jack || !list.contains(jack)) {
      rows().forEach(function (r) {
        r.removeAttribute("data-mark");
      });
      return paintDock();
    }
    var name = jack.querySelector(".w-name").textContent;
    var d = DATA[name];
    if (!d) return;

    if (e.metaKey || e.ctrlKey) {
      // The selected row is marked along with the clicked one, so the dock counts every lit row.
      var sel = list.querySelector(".w-jack[data-sel]");
      if (!list.querySelector(".w-jack[data-mark]") && sel && sel !== jack) {
        sel.setAttribute("data-mark", "");
      }
      jack.toggleAttribute("data-mark");
      return paintDock();
    }
    if (e.shiftKey) {
      var all = rows();
      var from = all.indexOf(list.querySelector(".w-jack[data-sel]"));
      var to = all.indexOf(jack);
      if (from < 0) from = to;
      all.slice(Math.min(from, to), Math.max(from, to) + 1).forEach(function (r) {
        r.setAttribute("data-mark", "");
      });
      return paintDock();
    }
    rows().forEach(function (r) {
      r.removeAttribute("data-mark");
    });
    paintDock();

    list.querySelectorAll(".w-jack[data-sel]").forEach(function (j) {
      j.removeAttribute("data-sel");
    });
    jack.setAttribute("data-sel", "");

    var os = jack.querySelector(".w-os");
    dname.innerHTML = os.innerHTML.replace('class="w-i"', "") + name;
    dname.querySelector("svg").style.color = os.style.color || "";
    ddesc.textContent = d.desc;
    target.innerHTML = d.target;
    route.innerHTML = d.route;
    fwrap.hidden = !d.fwd;
    if (d.fwd)
      fwrap.querySelector("dl").innerHTML = d.fwd
        .map(function (f) {
          return '<div class="w-row"><dt>' + f[0] + "</dt><dd>" + f[1] + "</dd></div>";
        })
        .join("");
    reach.innerHTML = reachHtml(d.reach);
    cmdtext.textContent = d.cmd;
  });
})();

// Mirrors hops() and ssh_args() in src-tauri/src/patchbay.rs. A test there pins this config to
// this argv, so a change to the flag order fails the build naming this page.
(function () {
  var out = document.getElementById("argv");
  if (!out) return;
  var fields = [].slice.call(document.querySelectorAll("[data-f]"));
  if (!fields.length) return;

  var unquote = function (v) {
    return v.trim().replace(/^"|"$/g, "");
  };

  function read() {
    var j = { defaults: {}, bastion: {}, web: {}, db: {} };
    fields.forEach(function (el) {
      var parts = el.dataset.f.split(".");
      j[parts[0]][parts[1]] = unquote(el.textContent);
    });
    ["bastion", "web", "db"].forEach(function (n) {
      if (!j[n].user) j[n].user = j.defaults.user; // [defaults] inheritance
    });
    return j;
  }

  var spec = function (k) {
    return k.user ? k.user + "@" + k.host : k.host;
  };

  // Walks jump outward from the target, guards against a loop, then reverses: ssh -J dials
  // the outermost bastion first.
  function hops(name, j) {
    var out = [],
      seen = {},
      hop = j[name] && j[name].jump;
    seen[name] = 1;
    while (hop) {
      if (seen[hop]) return { err: 'jump loop through "' + hop + '"' };
      seen[hop] = 1;
      var via = j[hop];
      if (!via) {
        out.push(hop);
        break;
      } // not a jack: a raw ssh spec
      out.push(spec(via) + (via.port ? ":" + via.port : ""));
      hop = via.jump;
    }
    return { hops: out.reverse() };
  }

  // Everything below reaches innerHTML, and every value was typed into the panel by the reader.
  function esc(t) {
    return String(t).replace(/[&<>]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c];
    });
  }

  function render() {
    var j = read(),
      chain = hops("db", j);
    if (chain.err) {
      out.innerHTML = '<span class="bad">' + esc(chain.err) + "</span>";
      return;
    }
    var html = '<span class="cmd">ssh</span> ';
    if (chain.hops.length)
      html += '<span class="f-chain">-J ' + esc(chain.hops.join(",")) + "</span> ";
    if (j.db.port) html += "-p " + esc(j.db.port) + " ";
    if (j.db.forward) html += '<span class="f-forward">-L ' + esc(j.db.forward) + "</span> ";
    var inherited = !fields.filter(function (e) {
      return e.dataset.f === "db.user";
    }).length;
    html +=
      inherited && j.db.user
        ? '<span class="f-default">' + esc(j.db.user) + "@</span>" + esc(j.db.host)
        : esc(spec(j.db));
    out.innerHTML = html;
  }

  fields.forEach(function (el) {
    el.addEventListener("input", render);
    // A value is one line; Enter would put a <br> in it.
    el.addEventListener("keydown", function (e) {
      if (e.key === "Enter") {
        e.preventDefault();
        el.blur();
      }
    });
    el.addEventListener("paste", function (e) {
      e.preventDefault();
      document.execCommand(
        "insertText",
        false,
        (e.clipboardData || window.clipboardData).getData("text").replace(/\s+/g, " "),
      );
    });
  });
  render();
})();

// Floats the build matching this machine to the top. The others stay listed: the architecture
// check is a heuristic.
(function () {
  var menu = document.querySelector(".dlmenu");
  if (!menu) return;
  var ua = navigator.userAgent || "";
  var os = /Mac/.test(ua) ? "mac" : /Win/.test(ua) ? "win" : /Linux|X11/.test(ua) ? "deb" : "";
  if (!os) return;

  function macArch() {
    // Chromium reports the architecture; elsewhere the GPU string is the only tell.
    try {
      var gl = document.createElement("canvas").getContext("webgl");
      var ext = gl && gl.getExtension("WEBGL_debug_renderer_info");
      var r = ext ? String(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)) : "";
      if (/Apple\s*(GPU|M\d)/i.test(r)) return "mac-arm";
      if (/Intel|Radeon|AMD/i.test(r)) return "mac-x64";
    } catch (e) {
      /* no webgl, no guess */
    }
    return "mac-arm";
  }

  // The button points at the releases page until the build is known, then straight at the file.
  var btn = menu.parentNode.querySelector(".btn");

  function mark(key) {
    var row = menu.querySelector('[data-for="' + key + '"]');
    if (!row) return;
    row.setAttribute("data-here", "");
    menu.insertBefore(row, menu.firstElementChild);
    if (btn) btn.href = row.href;
  }

  if (os !== "mac") return mark(os);
  var hi = navigator.userAgentData && navigator.userAgentData.getHighEntropyValues;
  if (!hi) return mark(macArch());
  navigator.userAgentData
    .getHighEntropyValues(["architecture"])
    .then(function (v) {
      mark(v && v.architecture === "x86" ? "mac-x64" : "mac-arm");
    })
    .catch(function () {
      mark(macArch());
    });
})();

// The changelog, fetched from the releases API when the section scrolls into view. Drafts are
// invisible without a token, so a release appears here when it is published.
(function () {
  var list = document.getElementById("log");
  var off = document.getElementById("log-off");
  if (!list) return;

  function esc(t) {
    return String(t).replace(/[&<>"]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c];
    });
  }

  // The shape the app parses: bullets that wrap, and `words` in backticks. Escaped first.
  function bullets(body) {
    var items = [];
    (body || "").split("\n").forEach(function (raw) {
      var line = raw.trim();
      if (!line || line.charAt(0) === "#") return;
      if (/^[-*] /.test(line)) items.push(line.slice(2));
      else if (items.length) items[items.length - 1] += " " + line;
      else items.push(line);
    });
    return items
      .map(function (t) {
        return "<li>" + esc(t).replace(/`([^`]+)`/g, "<code>$1</code>") + "</li>";
      })
      .join("");
  }

  var when = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "long", year: "numeric" });

  function draw(releases) {
    var rows = releases
      .filter(function (r) {
        return !r.draft;
      })
      .slice(0, 12);
    if (!rows.length) throw new Error("nothing published yet");
    list.innerHTML = rows
      .map(function (r) {
        var notes = bullets(r.body);
        return (
          '<li><div><span class="ver">' +
          esc(r.tag_name) +
          "</span>" +
          '<span class="when">' +
          esc(when.format(new Date(r.published_at))) +
          "</span></div>" +
          (notes ? "<ul>" + notes + "</ul>" : "<ul><li>No notes for this one.</li></ul>") +
          "</li>"
        );
      })
      .join("");
  }

  var asked = false;
  new IntersectionObserver(
    function (entries, self) {
      if (!entries[0].isIntersecting || asked) return;
      asked = true;
      self.disconnect();
      fetch("https://api.github.com/repos/pyvnoaim/patchbay/releases?per_page=12")
        .then(function (r) {
          return r.ok ? r.json() : Promise.reject(r.status);
        })
        .then(draw)
        .catch(function () {
          list.hidden = true;
          off.hidden = false;
        });
    },
    { rootMargin: "200px" },
  ).observe(list);
})();

// Sections arrive rather than appear, one element at a time. The attribute is set here and never
// in the markup, so a blocked script never leaves the page blank.
(function () {
  if (!window.IntersectionObserver) return;
  if (window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  var io = new IntersectionObserver(
    function (entries, self) {
      entries.forEach(function (e) {
        if (!e.isIntersecting) return;
        e.target.classList.add("in");
        self.unobserve(e.target);
      });
      // Held until the element is properly in the frame, so it reads as arriving, not flickering.
    },
    { rootMargin: "0px 0px -12% 0px", threshold: 0.1 },
  );

  // The text column is opened up so its lines move one at a time; a picture moves as one thing.
  // A spec list's rows are content, so they are opened up too.
  function units(wrap, out) {
    Array.prototype.forEach.call(wrap.children, function (el) {
      if (el.matches(".body, .col")) units(el, out);
      else if (el.matches(".spec")) units(el, out);
      else out.push(el);
    });
    return out;
  }

  document.querySelectorAll(".band > .wrap, .close > .wrap").forEach(function (wrap) {
    units(wrap, []).forEach(function (el, i) {
      // A spec row given the picture's travel makes the list wobble, so the kind is named here.
      var visual = el.matches(".full, .xform, .sheet, figure, img");
      el.setAttribute("data-rise", visual ? "visual" : "");
      // Capped: past half a dozen beats a stagger reads as lag.
      el.style.setProperty("--rise-d", Math.min(i, 6) * 70 + "ms");
      io.observe(el);
    });
  });
})();

// The patch panel: their fields on the left, ours on the right, and a lead measured between each
// pair so it survives a resize and a late font. The password lead is left hanging on purpose.
// Both columns read as plain lists before this runs; this only ever adds.
(function () {
  var panel = document.querySelector(".patch");
  var svg = panel && panel.querySelector(".cables");
  if (!panel || !svg) return;

  var PAIRS = [
    ["folder", "folders"],
    ["name", "jack"],
    ["uri", "host"],
    ["port", "rdp"],
    ["user", "who"],
  ];
  var NS = "http://www.w3.org/2000/svg";
  var slow = window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches;
  var leads = [],
    loose = null,
    looseSocket = null,
    plug = null,
    timer = null,
    playing = false;

  function pin(name) {
    return panel.querySelector('[data-pin="' + name + '"]');
  }

  // The socket, in the panel's coordinates: the file's outer edge, halfway down the row.
  function socket(el, right) {
    var p = panel.getBoundingClientRect();
    // The file's edge, not the row's: a lead has to arrive at the panel's border.
    var r = el.getBoundingClientRect(),
      f = el.closest(".file").getBoundingClientRect();
    return { x: (right ? f.right : f.left) - p.left, y: r.top - p.top + r.height / 2 };
  }

  // A real lead hangs: control points pulled past halfway so it leaves each socket horizontally,
  // and the sag grows with the drop.
  function curve(a, b) {
    var pull = Math.max((b.x - a.x) * 0.55, 26);
    var sag = Math.min(Math.abs(b.y - a.y) * 0.12 + 8, 26);
    return (
      "M" +
      a.x +
      " " +
      a.y +
      " C" +
      (a.x + pull) +
      " " +
      (a.y + sag) +
      " " +
      (b.x - pull) +
      " " +
      (b.y + sag) +
      " " +
      b.x +
      " " +
      b.y
    );
  }

  function draw() {
    var wide = panel.clientWidth > 0 && getComputedStyle(svg).display !== "none";
    if (!wide) return;
    svg.setAttribute("viewBox", "0 0 " + panel.clientWidth + " " + panel.clientHeight);
    svg.setAttribute("width", panel.clientWidth);
    svg.setAttribute("height", panel.clientHeight);

    leads.forEach(function (l) {
      var a = socket(l.from, true),
        b = socket(l.to, false);
      l.path.setAttribute("d", curve(a, b));
      l.a.setAttribute("cx", a.x);
      l.a.setAttribute("cy", a.y);
      l.b.setAttribute("cx", b.x);
      l.b.setAttribute("cy", b.y);
      var len = l.path.getTotalLength();
      l.len = len;
      l.path.style.strokeDasharray = len;
      // Held back until it is this lead's turn, unless motion is off.
      if (!l.anim) l.path.style.strokeDashoffset = slow || l.done ? 0 : len;
    });
    if (loose) {
      var a = socket(pin("pass"), true);
      looseSocket.setAttribute("cx", a.x);
      looseSocket.setAttribute("cy", a.y);
      var stop = { x: a.x + Math.min(panel.clientWidth * 0.16, 90), y: a.y + 34 };
      loose.setAttribute("d", curve(a, stop));
      plug.setAttribute("cx", stop.x);
      plug.setAttribute("cy", stop.y);
    }
  }

  function dot(cls) {
    var c = document.createElementNS(NS, "circle");
    c.setAttribute("class", cls);
    c.setAttribute("r", "3.5");
    svg.appendChild(c);
    return c;
  }

  PAIRS.forEach(function (p) {
    var from = pin(p[0]),
      to = pin(p[1]);
    if (!from || !to) return;
    var path = document.createElementNS(NS, "path");
    path.setAttribute("class", "lead");
    svg.appendChild(path);
    leads.push({ from: from, to: to, path: path, done: false, a: dot("socket"), b: dot("socket") });
  });
  if (pin("pass")) {
    loose = document.createElementNS(NS, "path");
    loose.setAttribute("class", "loose");
    svg.appendChild(loose);
    looseSocket = dot("socket");
    plug = dot("plug");
  }

  function reset() {
    leads.forEach(function (l) {
      if (l.anim) l.anim.cancel();
      l.anim = null;
      l.done = false;
      l.path.style.strokeDashoffset = l.len;
      l.to.classList.remove("lit");
      l.from.classList.remove("lit");
      l.a.classList.remove("on");
      l.b.classList.remove("on");
    });
  }

  function later(fn, ms) {
    timer = setTimeout(fn, ms);
  }

  // Pulled out the way they went in, a beat apart, before the panel starts over.
  function unplug(then) {
    var drawn = leads.filter(function (l) {
      return l.done;
    });
    if (!drawn.length) {
      reset();
      then();
      return;
    }
    drawn.forEach(function (l, i) {
      l.to.classList.remove("lit");
      l.b.classList.remove("on");
      if (l.anim) l.anim.cancel();
      l.path.style.strokeDashoffset = 0;
      l.anim = l.path.animate([{ strokeDashoffset: 0 }, { strokeDashoffset: l.len }], {
        duration: 460,
        delay: i * 60,
        easing: "cubic-bezier(.6,0,.4,1)",
        fill: "forwards",
      });
    });
    later(
      function () {
        reset();
        then();
      },
      460 + drawn.length * 60,
    );
  }

  function restart() {
    unplug(function () {
      at = 0;
      next();
    });
  }

  // One lead drawn in, its far end lit when it arrives. Animated rather than transitioned: a
  // transition needs a starting value from an earlier frame, and the first lead never had one.
  function plugIn(l, then) {
    l.done = true;
    l.from.classList.add("lit");
    l.a.classList.add("on");
    l.anim = l.path.animate([{ strokeDashoffset: l.len }, { strokeDashoffset: 0 }], {
      duration: 620,
      easing: "cubic-bezier(.22,.75,.28,1)",
      fill: "forwards",
    });
    // Before the animation ends: that easing spends its last third crawling the final pixel.
    return setTimeout(function () {
      l.to.classList.add("lit");
      l.b.classList.add("on");
      then();
    }, 420);
  }

  var at = 0;
  function next() {
    if (at >= leads.length) {
      // A beat with the whole panel patched, then it starts over.
      later(restart, 2600);
      return;
    }
    timer = plugIn(leads[at++], function () {
      later(next, 260);
    });
  }

  new ResizeObserver(draw).observe(panel);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(draw);
  draw();

  if (slow) {
    leads.forEach(function (l) {
      l.done = true;
      l.from.classList.add("lit");
      l.to.classList.add("lit");
    });
    return;
  }
  new IntersectionObserver(
    function (e) {
      if (e[0].isIntersecting === playing) return;
      playing = e[0].isIntersecting;
      clearTimeout(timer);
      if (playing) {
        reset();
        at = 0;
        draw();
        // A beat before the first lead, so the band has landed before a cable draws across it.
        timer = setTimeout(next, 700);
      } else reset();
    },
    { threshold: 0.25 },
  ).observe(panel);
})();
