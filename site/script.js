/* CRT plasma behind the hero. The shader is React Bits' CRTWarp, run on raw
   WebGL instead of React + three: it is one fullscreen quad, and this page has
   no bundler on purpose. Colours are read from the theme tokens, so it follows
   light and dark like everything else. */
(function () {
  var hero = document.querySelector('.hero');
  var canvas = document.getElementById('crt');
  if (!hero || !canvas) return;

  var slow = window.matchMedia('(prefers-reduced-motion: reduce)');
  var gl = canvas.getContext('webgl', { antialias: false, alpha: false, powerPreference: 'low-power' });
  if (!gl) { canvas.remove(); return; }

  var VERT = 'attribute vec2 p;varying vec2 vUv;void main(){vUv=p*0.5+0.5;gl_Position=vec4(p,0.0,1.0);}';

  var FRAG = [
    'precision highp float;',
    'varying vec2 vUv;',
    'uniform vec2 uResolution;',
    'uniform float uTime;',
    'uniform vec3 uColor;',
    'uniform vec3 uBackgroundColor;',
    'uniform float uCurvature, uScanlineStrength, uScanlineFrequency;',
    'uniform float uWaveAmplitude, uWaveFrequency, uBloom, uBloomRadius;',
    'uniform float uNoise, uVignette, uBrightness, uRgbShift, uMouseStrength;',
    'uniform vec2 uPointer;',

    'float hash21(vec2 p){p=fract(p*vec2(123.34,456.21));p+=dot(p,p+45.32);return fract(p.x*p.y);}',

    'vec2 crtCurve(vec2 uv, float radius){',
    '  vec2 p=(uv-0.5)*2.0;',
    '  float r=max(radius,1.415);',
    '  float cornerScale=r/sqrt(max(r*r-2.0,0.001));',
    '  p=r*p/sqrt(max(r*r-dot(p,p),0.001));',
    '  p/=cornerScale;',
    '  return p*0.5+0.5;',
    '}',

    'float plasma(vec2 uv, float t){',
    '  float f=max(uWaveFrequency/2.2,0.001);',
    '  uv=(uv-0.5)*f+0.5;',
    '  float scan=0.5-0.5*cos(uv.y*3.14159265*uScanlineFrequency);',
    '  scan=mix(1.0,scan,uScanlineStrength);',
    '  uv*=vec2(80.0,24.0); uv=ceil(uv); uv/=vec2(80.0,24.0);',
    '  float amp=uWaveAmplitude/0.28;',
    '  float v=0.0;',
    '  v+=0.7*sin(0.5*uv.x+t/5.0);',
    '  v+=3.0*sin(1.6*uv.y+t/5.0);',
    '  v+=sin(10.0*(uv.y*sin(t/2.0)+uv.x*cos(t/5.0))+t/2.0);',
    '  float cx=uv.x+0.5*sin(t/2.0);',
    '  float cy=uv.y+0.5*cos(t/4.0);',
    '  v+=0.4*sin(sqrt(100.0*cx*cx+100.0*cy*cy+1.0)+t);',
    '  v+=0.9*sin(sqrt(75.0*cx*cx+25.0*cy*cy+1.0)+t);',
    '  v-=1.4*sin(sqrt(256.0*cx*cx+25.0*cy*cy+1.0)+t);',
    '  v+=0.3*sin(0.5*uv.y+uv.x+sin(t));',
    '  return scan*floor(3.0*(0.5+0.499*sin(v*amp)))/3.0;',
    '}',

    'void main(){',
    '  float curveRadius=(1.1+0.42/max(uCurvature,0.001))*exp(-uPointer.y*uMouseStrength*0.4);',
    '  vec2 uv=crtCurve(vUv,curveRadius);',
    '  uv.x-=uPointer.x*uMouseStrength*0.035;',
    '  float s=plasma(uv,uTime);',
    '  float r=0.01*uBloomRadius;',
    '  float glow=s*0.2;',
    '  glow+=plasma(uv+vec2(r,0.0),uTime)*0.12;',
    '  glow+=plasma(uv-vec2(r,0.0),uTime)*0.12;',
    '  glow+=plasma(uv+vec2(0.0,r),uTime)*0.12;',
    '  glow+=plasma(uv-vec2(0.0,r),uTime)*0.12;',
    '  glow+=plasma(uv+vec2(r),uTime)*0.08;',
    '  glow+=plasma(uv-vec2(r),uTime)*0.08;',
    '  glow+=plasma(uv+vec2(r,-r),uTime)*0.08;',
    '  glow+=plasma(uv+vec2(-r,r),uTime)*0.08;',
    '  float red=plasma(uv+vec2(uRgbShift,0.0),uTime);',
    '  float blue=plasma(uv-vec2(uRgbShift,0.0),uTime);',
    '  vec3 wave=uColor*(0.3+s*0.7+glow*uBloom*0.65);',
    '  wave+=(vec3(red,s,blue)-s)*0.42;',
    '  float edge=clamp(1.0-dot(vUv-0.5,vUv-0.5)*2.0,0.0,1.0);',
    '  float fade=mix(1.0,smoothstep(0.0,1.0,edge),uVignette);',
    '  float mask=clamp(s*0.82+glow*0.52,0.0,1.0)*fade;',
    '  float grain=hash21(gl_FragCoord.xy+vec2(fract(uTime)*173.0));',
    '  wave=max(wave*uBrightness,vec3(0.0));',
    '  vec3 c=mix(uBackgroundColor,wave,mask);',
    '  c+=(grain-0.5)*uNoise;',
    '  gl_FragColor=vec4(clamp(c,0.0,1.0),1.0);',
    '}'
  ].join('\n');

  function compile(type, src) {
    var sh = gl.createShader(type);
    gl.shaderSource(sh, src); gl.compileShader(sh);
    if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) { gl.deleteShader(sh); return null; }
    return sh;
  }
  var vs = compile(gl.VERTEX_SHADER, VERT), fs = compile(gl.FRAGMENT_SHADER, FRAG);
  if (!vs || !fs) { canvas.remove(); return; }
  var prog = gl.createProgram();
  gl.attachShader(prog, vs); gl.attachShader(prog, fs); gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) { canvas.remove(); return; }
  gl.useProgram(prog);

  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  var loc = gl.getAttribLocation(prog, 'p');
  gl.enableVertexAttribArray(loc);
  gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

  var U = {};
  ['uResolution','uTime','uColor','uBackgroundColor','uCurvature','uScanlineStrength',
   'uScanlineFrequency','uWaveAmplitude','uWaveFrequency','uBloom','uBloomRadius','uNoise',
   'uVignette','uBrightness','uRgbShift','uPointer','uMouseStrength'
  ].forEach(function (n) { U[n] = gl.getUniformLocation(prog, n); });

  /* Tuned down from the component's demo values: this sits behind a headline,
     so it is dim, slow and vignetted rather than a screensaver. */
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
  /* Straight to display space, no linear round trip: the dark cells then match
     --ground exactly and the canvas has no visible edge against the page. */
  function put(u, name, fallback) {
    var v = (css.getPropertyValue(name) || '').trim() || fallback;
    var m = /^#?([0-9a-f]{6})$/i.exec(v);
    var n = m ? parseInt(m[1], 16) : parseInt(fallback.slice(1), 16);
    gl.uniform3f(u, (n >> 16 & 255) / 255, (n >> 8 & 255) / 255, (n & 255) / 255);
  }
  var light = false;
  function theme() {
    put(U.uColor, '--accent', '#6aa6ff');
    put(U.uBackgroundColor, '--ground', '#0c0d11');
    light = getComputedStyle(hero).getPropertyValue('color-scheme').indexOf('light') > -1
         || /^#?f/i.test((css.getPropertyValue('--ground') || '').trim());
    /* On a light ground the phosphor has to be quiet or it looks like a fault. */
    gl.uniform1f(U.uBrightness, light ? 0.5 : 0.8);
    canvas.style.opacity = light ? '0.5' : '0.72';
  }
  theme();
  new MutationObserver(theme).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  if (window.matchMedia) {
    var mq = window.matchMedia('(prefers-color-scheme: light)');
    (mq.addEventListener ? mq.addEventListener.bind(mq, 'change') : mq.addListener.bind(mq))(theme);
  }

  var dpr = Math.min(window.devicePixelRatio || 1, 1);
  function resize() {
    var w = Math.max(hero.clientWidth, 1), h = Math.max(hero.clientHeight, 1);
    canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.uniform2f(U.uResolution, canvas.width, canvas.height);
  }
  new ResizeObserver(resize).observe(hero);
  resize();

  var px = 0, py = 0, tx = 0, ty = 0;
  hero.addEventListener('pointermove', function (e) {
    var r = hero.getBoundingClientRect();
    tx = ((e.clientX - r.left) / Math.max(r.width, 1)) * 2 - 1;
    ty = -(((e.clientY - r.top) / Math.max(r.height, 1)) * 2 - 1);
  }, { passive: true });
  hero.addEventListener('pointerleave', function () { tx = 0; ty = 0; });

  function draw(t) {
    gl.uniform1f(U.uTime, t);
    gl.uniform2f(U.uPointer, px, py);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  /* Reduced motion gets one still frame: the effect is there, it just holds. */
  if (slow.matches) { draw(8.0); return; }

  var seen = true;
  new IntersectionObserver(function (e) { seen = e[0].isIntersecting; }).observe(hero);

  var time = 0, last = 0, prev = 0, INTERVAL = 1000 / 30;
  (function loop(now) {
    requestAnimationFrame(loop);
    if (!seen || document.hidden) { prev = now; return; }
    if (now - last < INTERVAL) return;
    last = now - ((now - last) % INTERVAL);
    time += Math.min((now - prev) / 1000, 0.1) * 0.35;
    prev = now;
    px += (tx - px) * 0.08; py += (ty - py) * 0.08;
    draw(time);
  })(0);
})();

/* The window tips towards whichever corner the pointer is nearest, which reads
   as a solid object catching the light rather than a flat picture. */
(function () {
  var shot = document.querySelector(".shot");
  var win = shot && shot.querySelector(".win");
  if (!win) return;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  var raf = 0, tx = 0, ty = 0;
  shot.addEventListener("pointermove", function (e) {
    var r = win.getBoundingClientRect();
    // -0.5 at one edge, +0.5 at the other
    tx = (e.clientX - r.left) / Math.max(r.width, 1) - 0.5;
    ty = (e.clientY - r.top) / Math.max(r.height, 1) - 0.5;
    if (!raf) raf = requestAnimationFrame(apply);
  }, { passive: true });

  function apply() {
    raf = 0;
    win.style.transform =
      "perspective(1600px) rotateX(" + (-ty * 4.5).toFixed(2) + "deg) rotateY(" +
      (tx * 6).toFixed(2) + "deg) scale(1.012)";
  }
  shot.addEventListener("pointerleave", function () {
    if (raf) { cancelAnimationFrame(raf); raf = 0; }
    win.style.transform = "";
  });
})();

/* Click a row, the detail pane follows - the one interaction that's actually the
   point of the app. Everything else in the window (tabs, chips) stays a picture;
   this is the only piece worth wiring up, and swapping text in a fixed-size pane
   can't misbehave the way restructuring the layout did. */
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
    return rows.map(function (r) { return "<div class=\"w-row\"><dt>" + r[0] + "</dt><dd>" + r[1] + "</dd></div>"; }).join("");
  }
  function hops(chain, dest) {
    var spans = ['<span><i class="pip"></i>this machine</span>'];
    chain.forEach(function (h) { spans.push('<span><i class="pip"></i>' + h + '<i class="arm">jump</i></span>'); });
    spans.push('<span class="last"><i class="pip"></i>' + dest + "</span>");
    return spans.join("");
  }

  var DATA = {
    bastion: {
      desc: "the way into infra", target: row("bastion.example", "root", ["port", "2222"]),
      route: hops([], "root@bastion.example:2222"), fwd: null,
      reach: { ok: true, via: "this machine", ms: 9 }, cmd: "ssh -p 2222 root@bastion.example"
    },
    web: {
      desc: "webserver, eu", target: row("10.0.0.4", "deploy"),
      route: hops(["bastion.example:2222"], "deploy@10.0.0.4"), fwd: null,
      reach: { ok: true, via: "bastion.example:2222", ms: 19 }, cmd: "ssh -J bastion.example:2222 deploy@10.0.0.4"
    },
    db: {
      desc: "primary, replicas behind it", target: row("10.0.0.5", "root"),
      route: hops(["bastion.example:2222", "deploy@10.0.0.4"], "root@10.0.0.5"),
      fwd: [["-L", "5432:localhost:5432"]],
      reach: { ok: true, via: "bastion.example:2222", ms: 24 },
      cmd: "ssh -J bastion.example:2222,deploy@10.0.0.4 -L 5432:localhost:5432 root@10.0.0.5"
    },
    "web-us": {
      desc: "webserver, us", target: row("10.1.0.4", "deploy"),
      route: hops([], "deploy@10.1.0.4"), fwd: null,
      reach: { ok: true, via: "this machine", ms: 61 }, cmd: "ssh deploy@10.1.0.4"
    },
    "db-us": {
      desc: "replica, us", target: row("10.1.0.5", "root"),
      route: hops([], "root@10.1.0.5"), fwd: null,
      reach: { down: true }, cmd: "ssh root@10.1.0.5"
    },
    "staging-web": {
      desc: "staging", target: row("10.2.0.4", "deploy"),
      route: hops([], "deploy@10.2.0.4"), fwd: null,
      reach: { unknown: true }, cmd: "ssh deploy@10.2.0.4"
    },
    nas: {
      desc: "Synology, backups land here", target: row("10.0.0.20", "root"),
      route: hops([], "root@10.0.0.20"), fwd: null,
      reach: { down: true }, cmd: "ssh root@10.0.0.20"
    },
    hypervisor: {
      desc: "Proxmox host", target: row("10.0.0.30", "root"),
      route: hops([], "root@10.0.0.30"), fwd: null,
      reach: { ok: true, via: "this machine", ms: 4 }, cmd: "ssh root@10.0.0.30"
    },
    gateway: {
      desc: "OPNsense, the edge", target: row("10.0.0.1", "root"),
      route: hops([], "root@10.0.0.1"), fwd: null,
      reach: { ok: true, via: "this machine", ms: 2 }, cmd: "ssh root@10.0.0.1"
    },
    pi: {
      desc: "homelab", target: row("10.0.0.40", "root"),
      route: hops([], "root@10.0.0.40"), fwd: null,
      reach: { down: true }, cmd: "ssh root@10.0.0.40"
    },
    "acme-app": {
      desc: "acme's app server", target: row("10.0.0.10", "deploy"),
      route: hops([], "deploy@10.0.0.10"), fwd: null,
      reach: { ok: true, via: "this machine", ms: 71 }, cmd: "ssh deploy@10.0.0.10"
    },
    "acme-dc": {
      desc: "acme's domain controller", target: row("10.0.0.50", "administrator"),
      route: hops([], "administrator@10.0.0.50"), fwd: null,
      reach: { ok: true, via: "this machine", ms: 68 }, cmd: "ssh administrator@10.0.0.50"
    }
  };

  function reachHtml(r) {
    if (r.unknown) return '<span style="color:var(--ink-faint)">not probed yet</span>';
    if (r.down) return '<span style="color:var(--down)">down</span> &middot; timed out';
    return '<span style="color:var(--up)">up</span> &middot; ' + r.via + " &middot; " + r.ms + "ms";
  }

  list.addEventListener("click", function (e) {
    var jack = e.target.closest(".w-jack");
    if (!jack || !list.contains(jack)) return;
    var name = jack.querySelector(".w-name").textContent;
    var d = DATA[name];
    if (!d) return;

    list.querySelectorAll(".w-jack[data-sel]").forEach(function (j) { j.removeAttribute("data-sel"); });
    jack.setAttribute("data-sel", "");

    var os = jack.querySelector(".w-os");
    dname.innerHTML = os.innerHTML.replace('class="w-i"', "") + name;
    dname.querySelector("svg").style.color = os.style.color || "";
    ddesc.textContent = d.desc;
    target.innerHTML = d.target;
    route.innerHTML = d.route;
    fwrap.hidden = !d.fwd;
    if (d.fwd) fwrap.querySelector("dl").innerHTML = d.fwd.map(function (f) {
      return "<div class=\"w-row\"><dt>" + f[0] + "</dt><dd>" + f[1] + "</dd></div>";
    }).join("");
    reach.innerHTML = reachHtml(d.reach);
    cmdtext.textContent = d.cmd;
  });
})();

/* Mirrors hops() and ssh_args() in src-tauri/src/patchbay.rs. It is a second
   implementation of the one thing that must not be wrong, so a test in that file
   pins this exact config to this exact argv: change the flag order there and the
   build fails naming this page. */
(function () {
  var out = document.getElementById("argv");
  if (!out) return;
  var fields = [].slice.call(document.querySelectorAll("[data-f]"));
  if (!fields.length) return;

  var unquote = function (v) { return v.trim().replace(/^"|"$/g, ""); };

  function read() {
    var j = { defaults: {}, bastion: {}, web: {}, db: {} };
    fields.forEach(function (el) {
      var parts = el.dataset.f.split(".");
      j[parts[0]][parts[1]] = unquote(el.textContent);
    });
    ["bastion", "web", "db"].forEach(function (n) {
      if (!j[n].user) j[n].user = j.defaults.user;   // [defaults] inheritance
    });
    return j;
  }

  var spec = function (k) { return k.user ? k.user + "@" + k.host : k.host; };

  /* Walks jump outward from the target, guards against a loop, then reverses:
     ssh -J dials the outermost bastion first. */
  function hops(name, j) {
    var out = [], seen = { }, hop = j[name] && j[name].jump;
    seen[name] = 1;
    while (hop) {
      if (seen[hop]) return { err: 'jump loop through "' + hop + '"' };
      seen[hop] = 1;
      var via = j[hop];
      if (!via) { out.push(hop); break; }          // not a jack: a raw ssh spec
      out.push(spec(via) + (via.port ? ":" + via.port : ""));
      hop = via.jump;
    }
    return { hops: out.reverse() };
  }

  // Everything below reaches innerHTML, and every value in it was typed into the
  // panel by whoever is reading the page.
  function esc(t) {
    return String(t).replace(/[&<>]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c];
    });
  }

  function render() {
    var j = read(), chain = hops("db", j);
    if (chain.err) { out.innerHTML = '<span class="bad">' + esc(chain.err) + "</span>"; return; }
    var html = '<span class="cmd">ssh</span> ';
    if (chain.hops.length) html += '<span class="f-chain">-J ' + esc(chain.hops.join(",")) + "</span> ";
    if (j.db.port) html += "-p " + esc(j.db.port) + " ";
    if (j.db.forward) html += '<span class="f-forward">-L ' + esc(j.db.forward) + "</span> ";
    var inherited = !fields.filter(function (e) { return e.dataset.f === "db.user"; }).length;
    html += inherited && j.db.user
      ? '<span class="f-default">' + esc(j.db.user) + "@</span>" + esc(j.db.host)
      : esc(spec(j.db));
    out.innerHTML = html;
  }

  fields.forEach(function (el) {
    el.addEventListener("input", render);
    // a value is one line; Enter would put a <br> in it
    el.addEventListener("keydown", function (e) { if (e.key === "Enter") { e.preventDefault(); el.blur(); } });
    el.addEventListener("paste", function (e) {
      e.preventDefault();
      document.execCommand("insertText", false, (e.clipboardData || window.clipboardData).getData("text").replace(/\s+/g, " "));
    });
  });
  render();
})();

/* Marks the build that matches the machine asking and floats it to the top.
   Every other build stays listed: the architecture check is a heuristic and a
   wrong guess must not hide the one you actually need. */
(function () {
  var menu = document.querySelector(".dlmenu");
  if (!menu) return;
  var ua = navigator.userAgent || "";
  var os = /Mac/.test(ua) ? "mac" : /Win/.test(ua) ? "win" : /Linux|X11/.test(ua) ? "deb" : "";
  if (!os) return;

  function macArch() {
    // Chromium hands the architecture over directly; everywhere else the GPU
    // string is the only tell, and Rosetta makes even that a guess.
    try {
      var gl = document.createElement("canvas").getContext("webgl");
      var ext = gl && gl.getExtension("WEBGL_debug_renderer_info");
      var r = ext ? String(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)) : "";
      if (/Apple\s*(GPU|M\d)/i.test(r)) return "mac-arm";
      if (/Intel|Radeon|AMD/i.test(r)) return "mac-x64";
    } catch (e) { /* no webgl, no guess */ }
    return "mac-arm";
  }

  // The button points at the releases page until we know better; once we do it
  // points straight at the file, so one click is the download.
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
  navigator.userAgentData.getHighEntropyValues(["architecture"]).then(function (v) {
    mark(v && v.architecture === "x86" ? "mac-x64" : "mac-arm");
  }).catch(function () { mark(macArch()); });
})();

/* The changelog, fetched from the releases API the first time the section is
   scrolled to. Drafts need a token to see, so a release appears here at the
   moment it is published - which is the same moment the app starts offering it.
   A failed fetch leaves the link to GitHub rather than an empty heading. */
(function () {
  var list = document.getElementById("log");
  var off = document.getElementById("log-off");
  if (!list) return;

  function esc(t) {
    return String(t).replace(/[&<>"]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c];
    });
  }

  /* The same shape the app parses: bullets that wrap onto the next line, and the
     odd `word` in backticks. Escaped first, marked up second. */
  function bullets(body) {
    var items = [];
    (body || "").split("\n").forEach(function (raw) {
      var line = raw.trim();
      if (!line || line.charAt(0) === "#") return;
      if (/^[-*] /.test(line)) items.push(line.slice(2));
      else if (items.length) items[items.length - 1] += " " + line;
      else items.push(line);
    });
    return items.map(function (t) {
      return "<li>" + esc(t).replace(/`([^`]+)`/g, "<code>$1</code>") + "</li>";
    }).join("");
  }

  var when = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "long", year: "numeric" });

  function draw(releases) {
    var rows = releases.filter(function (r) { return !r.draft; }).slice(0, 12);
    if (!rows.length) throw new Error("nothing published yet");
    list.innerHTML = rows.map(function (r) {
      var notes = bullets(r.body);
      return '<li><div><span class="ver">' + esc(r.tag_name) + "</span>" +
        '<span class="when">' + esc(when.format(new Date(r.published_at))) + "</span></div>" +
        (notes ? "<ul>" + notes + "</ul>" : "<ul><li>No notes for this one.</li></ul>") +
        "</li>";
    }).join("");
  }

  var asked = false;
  new IntersectionObserver(function (entries, self) {
    if (!entries[0].isIntersecting || asked) return;
    asked = true;
    self.disconnect();
    fetch("https://api.github.com/repos/pyvnoaim/patchbay/releases?per_page=12")
      .then(function (r) { return r.ok ? r.json() : Promise.reject(r.status); })
      .then(draw)
      .catch(function () { list.hidden = true; off.hidden = false; });
  }, { rootMargin: "200px" }).observe(list);
})();

/* Sections arrive rather than appear. Per element rather than per block: a headline
   lands, then its paragraph, then the picture of the thing they describe - one beat
   apart, so the page reads in the order it was written. The attribute is set here and
   never in the markup, because a page that hides its own content and then waits on an
   observer is one blocked script away from being blank. Reduced motion never gets as
   far as setting it. */
(function () {
  if (!window.IntersectionObserver) return;
  if (window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  var io = new IntersectionObserver(function (entries, self) {
    entries.forEach(function (e) {
      if (!e.isIntersecting) return;
      e.target.classList.add("in");
      self.unobserve(e.target);
    });
  // Held until the element is properly in the frame rather than a pixel over the edge,
  // which is what makes it read as arriving rather than as a flicker at the fold.
  }, { rootMargin: "0px 0px -12% 0px", threshold: 0.1 });

  // The text column is opened up so its lines move one at a time; everything beside it
  // is a picture and moves as one thing. A feature list is the exception worth making:
  // its rows are the content, not a container of it.
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
      // Named rather than inferred: a spec row is an icon and two lines of text, and
      // giving it the picture's travel makes the list wobble instead of settle.
      var visual = el.matches(".full, .xform, .sheet, figure, img");
      el.setAttribute("data-rise", visual ? "visual" : "");
      // Capped: past half a dozen beats a stagger stops reading as sequence and starts
      // reading as lag, and a spec list has eight rows.
      el.style.setProperty("--rise-d", (Math.min(i, 6) * 70) + "ms");
      io.observe(el);
    });
  });
})();

/* The patch panel. Their field on the left, ours on the right, and a lead drawn
   between each pair - measured from where the rows actually are, so it survives a
   resize, a late font and a language that wraps differently. One lead is left hanging
   in the gap: the password, which patchbay has nowhere to put on purpose.

   Both columns read as plain lists before this runs, and this only ever adds. */
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
  var leads = [], loose = null, looseSocket = null, plug = null, timer = null, playing = false;

  function pin(name) { return panel.querySelector('[data-pin="' + name + '"]'); }

  /// The socket, in the panel's own coordinates: the outer edge of the row, halfway
  /// down it. Read rather than assumed, because the two columns are different heights
  /// and the rows are wherever the text put them.
  function socket(el, right) {
    var p = panel.getBoundingClientRect();
    // The row's own height for the vertical, but the *file's* edge for the horizontal:
    // a line runs the width of the panel, and a lead has to arrive at the panel's
    // border rather than at wherever that line's text happens to stop.
    var r = el.getBoundingClientRect(), f = el.closest(".file").getBoundingClientRect();
    return { x: (right ? f.right : f.left) - p.left, y: r.top - p.top + r.height / 2 };
  }

  /// A real lead hangs. The control points are pulled well past halfway so the curve
  /// leaves each socket horizontally, and the sag grows with the drop - a patch across
  /// two rows droops less than one across six.
  function curve(a, b) {
    var pull = Math.max((b.x - a.x) * 0.55, 26);
    var sag = Math.min(Math.abs(b.y - a.y) * 0.12 + 8, 26);
    return "M" + a.x + " " + a.y +
      " C" + (a.x + pull) + " " + (a.y + sag) +
      " " + (b.x - pull) + " " + (b.y + sag) +
      " " + b.x + " " + b.y;
  }

  function draw() {
    var wide = panel.clientWidth > 0 && getComputedStyle(svg).display !== "none";
    if (!wide) return;
    svg.setAttribute("viewBox", "0 0 " + panel.clientWidth + " " + panel.clientHeight);
    svg.setAttribute("width", panel.clientWidth);
    svg.setAttribute("height", panel.clientHeight);

    leads.forEach(function (l) {
      var a = socket(l.from, true), b = socket(l.to, false);
      l.path.setAttribute("d", curve(a, b));
      l.a.setAttribute("cx", a.x); l.a.setAttribute("cy", a.y);
      l.b.setAttribute("cx", b.x); l.b.setAttribute("cy", b.y);
      var len = l.path.getTotalLength();
      l.len = len;
      l.path.style.strokeDasharray = len;
      // Held back until it is this lead's turn, unless motion is off - then they are
      // all simply plugged in, which is the same picture without the theatre.
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
    var from = pin(p[0]), to = pin(p[1]);
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

  var at = 0;
  function next() {
    if (at >= leads.length) {
      // A beat with the whole panel patched, then it starts over.
      timer = setTimeout(function () { reset(); at = 0; next(); }, 2600);
      return;
    }
    var l = leads[at++];
    l.from.classList.add("lit");
    l.a.classList.add("on");
    // Animated rather than transitioned: a transition needs the browser to have seen
    // the starting value in an earlier frame, and the first lead never got one - it
    // was reset and told to draw inside the same batch, so it simply appeared. An
    // animation carries its own from and to and cannot be coalesced away.
    l.done = true;
    l.anim = l.path.animate(
      [{ strokeDashoffset: l.len }, { strokeDashoffset: 0 }],
      { duration: 620, easing: "cubic-bezier(.22,.75,.28,1)", fill: "forwards" }
    );
    // Lit when the lead actually arrives, not when it sets off.
    timer = setTimeout(function () {
      l.to.classList.add("lit");
      l.b.classList.add("on");
      timer = setTimeout(next, 260);
    }, 620);
  }

  new ResizeObserver(draw).observe(panel);
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(draw);
  draw();

  if (slow) {
    leads.forEach(function (l) { l.from.classList.add("lit"); l.to.classList.add("lit"); });
    return;
  }
  new IntersectionObserver(function (e) {
    if (e[0].isIntersecting === playing) return;
    playing = e[0].isIntersecting;
    clearTimeout(timer);
    if (playing) {
      reset();
      at = 0;
      draw();
      // A beat before the first lead. The band is still rising in when it crosses the
      // threshold, and a cable drawing itself across a panel that has not landed yet
      // reads as two animations fighting rather than one following the other.
      timer = setTimeout(next, 700);
    }
    else reset();
  }, { threshold: 0.25 }).observe(panel);
})();
