/* Pure WebGL2 + core WASM Parasolid XT Viewer & Mass Calculator */

let wasm = null;
let currentFilename = "model";
let renderMode = "shaded_edges";

const canvas = document.getElementById("gl-canvas");
const gl = canvas.getContext("webgl2", { antialias: true, alpha: true });

// Camera orbital state
const cam = {
  target: [0, 0, 0],
  distance: 80,
  yaw: 0.72,
  pitch: 0.52,
  modelRadius: 30,
};

let scene = {
  posBuf: null,
  normBuf: null,
  colBuf: null,
  idxBuf: null,
  triCount: 0,
  edgeBuf: null,
  edgeVertCount: 0,
  gridBuf: null,
  gridVertCount: 0,
};

// Shaders
const MESH_VS = `#version 300 es
precision highp float;
layout(location=0) in vec3 aPos;
layout(location=1) in vec3 aNorm;
layout(location=2) in vec3 aCol;
uniform mat4 uMvp;
uniform mat4 uView;
out vec3 vNormView;
out vec3 vPosView;
out vec3 vCol;
void main() {
  vec4 pv = uView * vec4(aPos, 1.0);
  vPosView = pv.xyz;
  vNormView = mat3(uView) * aNorm;
  vCol = aCol;
  gl_Position = uMvp * vec4(aPos, 1.0);
}`;

const MESH_FS = `#version 300 es
precision highp float;
in vec3 vNormView;
in vec3 vPosView;
in vec3 vCol;
uniform float uAlpha;
out vec4 fragColor;
void main() {
  vec3 N = normalize(vNormView);
  if (!gl_FrontFacing) N = -N;
  vec3 V = normalize(-vPosView);
  vec3 L1 = normalize(vec3(0.55, 0.72, 0.85));
  vec3 L2 = normalize(vec3(-0.65, -0.35, 0.35));
  vec3 H1 = normalize(L1 + V);
  float diff1 = max(dot(N, L1), 0.0);
  float diff2 = max(dot(N, L2), 0.0);
  float spec = pow(max(dot(N, H1), 0.0), 42.0) * 0.25;
  vec3 lit = vCol * (0.28 + 0.62 * diff1 + 0.22 * diff2) + vec3(spec);
  fragColor = vec4(clamp(lit, 0.0, 1.0), uAlpha);
}`;

const LINE_VS = `#version 300 es
precision highp float;
layout(location=0) in vec3 aPos;
uniform mat4 uMvp;
void main() {
  gl_Position = uMvp * vec4(aPos, 1.0);
  gl_Position.z -= 0.0004 * gl_Position.w;
}`;

const LINE_FS = `#version 300 es
precision highp float;
uniform vec4 uColor;
out vec4 fragColor;
void main() {
  fragColor = uColor;
}`;

let progMesh = null;
let progLine = null;

function compileShader(type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    throw new Error(gl.getShaderInfoLog(s));
  }
  return s;
}

function linkProgram(vs, fs) {
  const p = gl.createProgram();
  gl.attachShader(p, compileShader(gl.VERTEX_SHADER, vs));
  gl.attachShader(p, compileShader(gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    throw new Error(gl.getProgramInfoLog(p));
  }
  return p;
}

// Matrix math (column-major Float32Array)
function mat4Mul(a, b) {
  const out = new Float32Array(16);
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      out[c * 4 + r] =
        a[0 * 4 + r] * b[c * 4 + 0] +
        a[1 * 4 + r] * b[c * 4 + 1] +
        a[2 * 4 + r] * b[c * 4 + 2] +
        a[3 * 4 + r] * b[c * 4 + 3];
    }
  }
  return out;
}

function mat4Persp(fovy, aspect, near, far) {
  const f = 1.0 / Math.tan(fovy * 0.5);
  const nf = 1.0 / (near - far);
  const m = new Float32Array(16);
  m[0] = f / aspect;
  m[5] = f;
  m[10] = (far + near) * nf;
  m[11] = -1;
  m[14] = 2 * far * near * nf;
  return m;
}

function normVec(v) {
  const l = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / l, v[1] / l, v[2] / l];
}

function crossVec(a, b) {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}

function mat4LookAt(eye, target, up) {
  const z = normVec([eye[0] - target[0], eye[1] - target[1], eye[2] - target[2]]);
  const x = normVec(crossVec(up, z));
  const y = crossVec(z, x);
  const m = new Float32Array(16);
  m[0] = x[0]; m[4] = x[1]; m[8] = x[2];
  m[1] = y[0]; m[5] = y[1]; m[9] = y[2];
  m[2] = z[0]; m[6] = z[1]; m[10] = z[2];
  m[12] = -(x[0] * eye[0] + x[1] * eye[1] + x[2] * eye[2]);
  m[13] = -(y[0] * eye[0] + y[1] * eye[1] + y[2] * eye[2]);
  m[14] = -(z[0] * eye[0] + z[1] * eye[1] + z[2] * eye[2]);
  m[15] = 1;
  return m;
}

function initGL() {
  if (!gl) return;
  progMesh = linkProgram(MESH_VS, MESH_FS);
  progLine = linkProgram(LINE_VS, LINE_FS);
  buildReferenceGrid(100, 10);
}

function buildReferenceGrid(extent, step) {
  const pts = [];
  for (let i = -extent; i <= extent; i += step) {
    pts.push(i, -extent, 0, i, extent, 0);
    pts.push(-extent, i, 0, extent, i, 0);
  }
  scene.gridBuf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, scene.gridBuf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(pts), gl.STATIC_DRAW);
  scene.gridVertCount = pts.length / 3;
}

function draw() {
  if (!gl || !progMesh) return;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const w = Math.max(1, Math.floor(canvas.clientWidth * dpr));
  const h = Math.max(1, Math.floor(canvas.clientHeight * dpr));
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  gl.viewport(0, 0, w, h);
  gl.clearColor(0.045, 0.072, 0.135, 1.0);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  gl.enable(gl.DEPTH_TEST);

  const cp = Math.cos(cam.pitch), sp = Math.sin(cam.pitch);
  const cy = Math.cos(cam.yaw), sy = Math.sin(cam.yaw);
  const eye = [
    cam.target[0] + cam.distance * cp * cy,
    cam.target[1] + cam.distance * cp * sy,
    cam.target[2] + cam.distance * sp,
  ];
  const view = mat4LookAt(eye, cam.target, [0, 0, 1]);
  const near = Math.max(0.05, cam.distance * 0.005);
  const far = Math.max(500, cam.distance * 25.0);
  const proj = mat4Persp((40 * Math.PI) / 180, w / h, near, far);
  const mvp = mat4Mul(proj, view);

  // Reference XY grid
  if (scene.gridBuf && scene.gridVertCount > 0) {
    gl.useProgram(progLine);
    gl.uniformMatrix4fv(gl.getUniformLocation(progLine, "uMvp"), false, mvp);
    gl.uniform4f(gl.getUniformLocation(progLine, "uColor"), 0.25, 0.34, 0.48, 0.28);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.bindBuffer(gl.ARRAY_BUFFER, scene.gridBuf);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    gl.drawArrays(gl.LINES, 0, scene.gridVertCount);
    gl.disableVertexAttribArray(0);
    gl.disable(gl.BLEND);
  }

  // Shaded triangles
  if (scene.triCount > 0 && renderMode !== "wireframe") {
    gl.useProgram(progMesh);
    gl.uniformMatrix4fv(gl.getUniformLocation(progMesh, "uMvp"), false, mvp);
    gl.uniformMatrix4fv(gl.getUniformLocation(progMesh, "uView"), false, view);
    const alpha = renderMode === "xray" ? 0.35 : 1.0;
    gl.uniform1f(gl.getUniformLocation(progMesh, "uAlpha"), alpha);
    if (alpha < 1.0) {
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      gl.depthMask(false);
    }

    gl.bindBuffer(gl.ARRAY_BUFFER, scene.posBuf);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);

    gl.bindBuffer(gl.ARRAY_BUFFER, scene.normBuf);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 3, gl.FLOAT, false, 0, 0);

    gl.bindBuffer(gl.ARRAY_BUFFER, scene.colBuf);
    gl.enableVertexAttribArray(2);
    gl.vertexAttribPointer(2, 3, gl.FLOAT, false, 0, 0);

    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, scene.idxBuf);
    gl.drawElements(gl.TRIANGLES, scene.triCount, gl.UNSIGNED_INT, 0);

    gl.disableVertexAttribArray(0);
    gl.disableVertexAttribArray(1);
    gl.disableVertexAttribArray(2);
    if (alpha < 1.0) {
      gl.depthMask(true);
      gl.disable(gl.BLEND);
    }
  }

  // B-rep edges
  if (scene.edgeVertCount > 0 && (renderMode === "shaded_edges" || renderMode === "wireframe" || renderMode === "xray")) {
    gl.useProgram(progLine);
    gl.uniformMatrix4fv(gl.getUniformLocation(progLine, "uMvp"), false, mvp);
    if (renderMode === "wireframe") {
      gl.uniform4f(gl.getUniformLocation(progLine, "uColor"), 0.22, 0.82, 0.96, 1.0);
    } else {
      gl.uniform4f(gl.getUniformLocation(progLine, "uColor"), 0.06, 0.09, 0.14, 0.92);
    }
    gl.bindBuffer(gl.ARRAY_BUFFER, scene.edgeBuf);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    gl.drawArrays(gl.LINES, 0, scene.edgeVertCount);
    gl.disableVertexAttribArray(0);
  }
}

// WASM result reader helper: [u32 len | payload bytes]
function readWasmBytes(ptr) {
  const mem = wasm.exports.memory.buffer;
  const view = new DataView(mem);
  const len = view.getUint32(ptr, true);
  return new Uint8Array(mem, ptr + 4, len).slice();
}

function readWasmJson(ptr) {
  const raw = readWasmBytes(ptr);
  return JSON.parse(new TextDecoder().decode(raw));
}

function uploadGeometry(bufs, mass) {
  const mem = wasm.exports.memory.buffer;
  const posArr = new Float32Array(mem, bufs.pos_ptr, bufs.pos_len);
  const normArr = new Float32Array(mem, bufs.norm_ptr, bufs.norm_len);
  const colArr = new Float32Array(mem, bufs.col_ptr, bufs.col_len);
  const idxArr = new Uint32Array(mem, bufs.idx_ptr, bufs.idx_len);
  const edgeArr = new Float32Array(mem, bufs.edge_ptr, bufs.edge_len);

  if (scene.posBuf) gl.deleteBuffer(scene.posBuf);
  if (scene.normBuf) gl.deleteBuffer(scene.normBuf);
  if (scene.colBuf) gl.deleteBuffer(scene.colBuf);
  if (scene.idxBuf) gl.deleteBuffer(scene.idxBuf);
  if (scene.edgeBuf) gl.deleteBuffer(scene.edgeBuf);

  scene.posBuf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, scene.posBuf);
  gl.bufferData(gl.ARRAY_BUFFER, posArr, gl.STATIC_DRAW);

  scene.normBuf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, scene.normBuf);
  gl.bufferData(gl.ARRAY_BUFFER, normArr, gl.STATIC_DRAW);

  scene.colBuf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, scene.colBuf);
  gl.bufferData(gl.ARRAY_BUFFER, colArr, gl.STATIC_DRAW);

  scene.idxBuf = gl.createBuffer();
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, scene.idxBuf);
  gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, idxArr, gl.STATIC_DRAW);
  scene.triCount = idxArr.length;

  scene.edgeBuf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, scene.edgeBuf);
  gl.bufferData(gl.ARRAY_BUFFER, edgeArr, gl.STATIC_DRAW);
  scene.edgeVertCount = edgeArr.length / 3;

  if (mass && mass.bbox_min_mm && mass.bbox_max_mm) {
    const mn = mass.bbox_min_mm;
    const mx = mass.bbox_max_mm;
    cam.target = [0.5 * (mn[0] + mx[0]), 0.5 * (mn[1] + mx[1]), 0.5 * (mn[2] + mx[2])];
    const diag = Math.hypot(mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]) || 20;
    cam.modelRadius = diag * 0.5;
    cam.distance = diag * 1.8;
  }
  draw();
}

function fmtNum(v, digits = 2) {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  if (Math.abs(v) >= 1e5 || (Math.abs(v) > 0 && Math.abs(v) < 1e-3)) return v.toExponential(3);
  return v.toLocaleString(undefined, { maximumFractionDigits: digits });
}

function renderMass(m) {
  const badge = document.getElementById("watertight-badge");
  if (!m) {
    badge.className = "pill pill-neutral";
    badge.textContent = "—";
    return;
  }
  if (m.is_closed) {
    badge.className = "pill pill-ok";
    badge.textContent = "Watertight Solid";
  } else {
    badge.className = "pill pill-warn";
    badge.textContent = "Open Sheet";
  }
  document.getElementById("m-euler").textContent = String(m.euler);
  document.getElementById("m-mass").textContent =
    m.mass_kg >= 1.0 ? `${fmtNum(m.mass_kg, 3)} kg` : `${fmtNum(m.mass_g, 2)} g`;
  document.getElementById("m-volume").textContent =
    m.is_closed
      ? (m.volume_cm3 >= 1.0 ? `${fmtNum(m.volume_cm3, 3)} cm³` : `${fmtNum(m.volume_mm3, 2)} mm³`)
      : "0 (open)";
  document.getElementById("m-area").textContent =
    m.area_cm2 >= 1.0 ? `${fmtNum(m.area_cm2, 3)} cm²` : `${fmtNum(m.area_mm2, 2)} mm²`;

  const dx = m.bbox_max_mm[0] - m.bbox_min_mm[0];
  const dy = m.bbox_max_mm[1] - m.bbox_min_mm[1];
  const dz = m.bbox_max_mm[2] - m.bbox_min_mm[2];
  document.getElementById("m-size").textContent = `${fmtNum(dx, 2)} × ${fmtNum(dy, 2)} × ${fmtNum(dz, 2)}`;
  document.getElementById("m-com").textContent =
    `(${fmtNum(m.centroid_mm[0], 2)}, ${fmtNum(m.centroid_mm[1], 2)}, ${fmtNum(m.centroid_mm[2], 2)})`;
  document.getElementById("m-inertia").textContent = m.principal_moments_g_mm2
    .map((x) => fmtNum(x, 1))
    .join(" / ");
}

function loadBytes(bytes, label) {
  if (!wasm) return;
  const status = document.getElementById("status-msg");
  status.textContent = "";
  const density = parseFloat(document.getElementById("density-input").value) || 2700;

  const ptr = wasm.exports.alloc(bytes.length);
  new Uint8Array(wasm.exports.memory.buffer, ptr, bytes.length).set(bytes);
  const resPtr = wasm.exports.load_xt(ptr, bytes.length, density);
  wasm.exports.dealloc(ptr, bytes.length);

  const res = readWasmJson(resPtr);
  if (!res.ok) {
    status.textContent = res.error || "Failed to parse Parasolid file";
    return;
  }

  currentFilename = label.replace(/\.[^.]+$/, "") || "xtopen_model";
  document.getElementById("model-name").textContent = label;
  document.getElementById("m-format").textContent = res.summary.format;
  document.getElementById("m-schema").textContent = res.summary.schema;
  document.getElementById("m-bodies").textContent =
    res.summary.instances > 0 ? `${res.summary.bodies} (${res.summary.instances} inst)` : String(res.summary.bodies);
  document.getElementById("m-assemblies").textContent = String(res.summary.assemblies);
  document.getElementById("m-faces").textContent = String(res.summary.faces);
  document.getElementById("m-edges").textContent = String(res.summary.edges);
  document.getElementById("m-tris").textContent = String(res.summary.triangles);

  if (res.summary.warnings && res.summary.warnings.length > 0) {
    status.textContent = `Note: ${res.summary.warnings[0]}`;
  }

  renderMass(res.mass);
  uploadGeometry(res.buffers, res.mass);

  for (const id of ["dl-step", "dl-stl", "dl-obj", "dl-xt", "dl-xb"]) {
    document.getElementById(id).disabled = false;
  }
}

async function loadSample(url, btn) {
  document.querySelectorAll(".chip").forEach((c) => c.classList.remove("active"));
  if (btn) btn.classList.add("active");
  const resp = await fetch(url);
  const buf = new Uint8Array(await resp.arrayBuffer());
  loadBytes(buf, url.split("/").pop());
}

function downloadBlob(bytes, ext, mime) {
  const blob = new Blob([bytes], { type: mime });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `${currentFilename}.${ext}`;
  document.body.appendChild(a);
  a.click();
  setTimeout(() => {
    URL.revokeObjectURL(a.href);
    a.remove();
  }, 100);
}

// Orbit / Pan / Zoom interactions
function wireControls() {
  let dragging = false;
  let panMode = false;
  let lastX = 0, lastY = 0;

  canvas.addEventListener("contextmenu", (e) => e.preventDefault());
  canvas.addEventListener("pointerdown", (e) => {
    dragging = true;
    panMode = e.button === 2 || e.button === 1 || e.shiftKey;
    lastX = e.clientX;
    lastY = e.clientY;
    canvas.setPointerCapture(e.pointerId);
  });
  canvas.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    const dx = e.clientX - lastX;
    const dy = e.clientY - lastY;
    lastX = e.clientX;
    lastY = e.clientY;
    if (panMode || e.shiftKey) {
      const cp = Math.cos(cam.pitch), sp = Math.sin(cam.pitch);
      const cy = Math.cos(cam.yaw), sy = Math.sin(cam.yaw);
      const viewDir = normVec([-cp * cy, -cp * sy, -sp]);
      const right = normVec(crossVec(viewDir, [0, 0, 1]));
      const up = crossVec(right, viewDir);
      const scale = cam.distance * 0.0016;
      cam.target[0] += (-dx * right[0] + dy * up[0]) * scale;
      cam.target[1] += (-dx * right[1] + dy * up[1]) * scale;
      cam.target[2] += (-dx * right[2] + dy * up[2]) * scale;
    } else {
      cam.yaw -= dx * 0.008;
      cam.pitch = Math.max(-1.52, Math.min(1.52, cam.pitch + dy * 0.008));
    }
    draw();
  });
  canvas.addEventListener("pointerup", (e) => {
    dragging = false;
    try { canvas.releasePointerCapture(e.pointerId); } catch {}
  });
  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 1.12 : 0.89;
    cam.distance = Math.max(0.5, Math.min(50000, cam.distance * factor));
    draw();
  }, { passive: false });

  window.addEventListener("resize", draw);

  // Drag & Drop
  const dz = document.getElementById("drop-zone");
  const ov = document.getElementById("drop-overlay");
  dz.addEventListener("dragover", (e) => {
    e.preventDefault();
    ov.classList.remove("hidden");
  });
  dz.addEventListener("dragleave", () => ov.classList.add("hidden"));
  dz.addEventListener("drop", async (e) => {
    e.preventDefault();
    ov.classList.add("hidden");
    const file = e.dataTransfer?.files?.[0];
    if (file) {
      const buf = new Uint8Array(await file.arrayBuffer());
      loadBytes(buf, file.name);
    }
  });

  document.getElementById("file-input").addEventListener("change", async (e) => {
    const file = e.target.files?.[0];
    if (file) {
      const buf = new Uint8Array(await file.arrayBuffer());
      loadBytes(buf, file.name);
    }
  });

  document.querySelectorAll(".chip[data-sample]").forEach((btn) => {
    btn.addEventListener("click", () => loadSample(btn.dataset.sample, btn));
  });

  document.querySelectorAll(".tool-btn[data-view]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const v = btn.dataset.view;
      if (v === "iso") { cam.yaw = 0.72; cam.pitch = 0.52; }
      else if (v === "front") { cam.yaw = -Math.PI / 2; cam.pitch = 0.0; }
      else if (v === "top") { cam.yaw = -Math.PI / 2; cam.pitch = 1.50; }
      else if (v === "right") { cam.yaw = 0.0; cam.pitch = 0.0; }
      draw();
    });
  });

  document.getElementById("btn-fit").addEventListener("click", () => {
    cam.distance = cam.modelRadius * 3.6;
    draw();
  });

  document.getElementById("render-mode").addEventListener("change", (e) => {
    renderMode = e.target.value;
    draw();
  });

  const matSel = document.getElementById("material-preset");
  const densIn = document.getElementById("density-input");
  const updateDensity = () => {
    if (!wasm) return;
    const d = parseFloat(densIn.value);
    if (d > 0) {
      const res = readWasmJson(wasm.exports.recompute_mass(d));
      if (res.ok) renderMass(res.mass);
    }
  };
  matSel.addEventListener("change", () => {
    if (matSel.value !== "custom") {
      densIn.value = matSel.value;
      updateDensity();
    }
  });
  densIn.addEventListener("input", () => {
    matSel.value = "custom";
    updateDensity();
  });

  document.getElementById("dl-step").addEventListener("click", () => {
    const sc = parseInt(document.getElementById("step-schema").value, 10) || 0;
    const un = parseInt(document.getElementById("step-units").value, 10) || 0;
    downloadBlob(readWasmBytes(wasm.exports.export_step(sc, un)), "step", "application/step");
  });
  document.getElementById("dl-stl").addEventListener("click", () => {
    downloadBlob(readWasmBytes(wasm.exports.export_stl()), "stl", "application/octet-stream");
  });
  document.getElementById("dl-obj").addEventListener("click", () => {
    downloadBlob(readWasmBytes(wasm.exports.export_obj()), "obj", "text/plain");
  });
  document.getElementById("dl-xt").addEventListener("click", () => {
    downloadBlob(readWasmBytes(wasm.exports.transcode_xt(0)), "x_t", "text/plain");
  });
  document.getElementById("dl-xb").addEventListener("click", () => {
    downloadBlob(readWasmBytes(wasm.exports.transcode_xt(1)), "x_b", "application/octet-stream");
  });
}

async function boot() {
  initGL();
  wireControls();
  try {
    const resp = await fetch("xtopen_web.wasm");
    const bytes = await resp.arrayBuffer();
    const inst = await WebAssembly.instantiate(bytes, {});
    wasm = inst.instance;
    const firstBtn = document.querySelector(".chip[data-sample]");
    await loadSample("samples/plate_with_hole.x_b", firstBtn);
  } catch (err) {
    document.getElementById("model-name").textContent = "Failed to load WASM";
    document.getElementById("status-msg").textContent = String(err);
  }
}

boot();
