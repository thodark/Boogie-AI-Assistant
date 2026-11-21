use eframe::glow::{self, HasContext};

#[derive(Clone, Copy, Debug)]
pub struct OrbUniforms {
    pub time: f32,
    pub orb_center: [f32; 2],
    pub orb_scale: f32,
    pub audio: f32,
    pub openness: f32,
    pub width: f32,
    pub state_weights: [f32; 4], // [listening, speaking, thinking, sleeping]
    pub mouse: [f32; 2],
    pub muted: f32,
    pub pulse: f32,
}

pub struct OrbRenderer {
    program: glow::Program,
    vao: glow::VertexArray,
    u_viewport_origin: Option<glow::UniformLocation>,
    u_resolution: Option<glow::UniformLocation>,
    u_time: Option<glow::UniformLocation>,
    u_orb_center: Option<glow::UniformLocation>,
    u_orb_scale: Option<glow::UniformLocation>,
    u_audio: Option<glow::UniformLocation>,
    u_openness: Option<glow::UniformLocation>,
    u_width: Option<glow::UniformLocation>,
    u_state_weights: Option<glow::UniformLocation>,
    u_mouse: Option<glow::UniformLocation>,
    u_muted: Option<glow::UniformLocation>,
    u_pulse: Option<glow::UniformLocation>,
}

const VERT_BODY: &str = r#"
void main() {
    // Fullscreen triangle without any VBO
    vec2 pos = vec2(
        float((gl_VertexID & 1) << 2) - 1.0,
        float((gl_VertexID & 2) << 1) - 1.0
    );
    gl_Position = vec4(pos, 0.0, 1.0);
}
"#;

const FRAG_BODY: &str = r#"
out vec4 fragColor;

uniform vec2  u_viewport_origin;
uniform vec2  u_resolution;
uniform float u_time;
uniform vec2  u_orb_center;
uniform float u_orb_scale;
uniform float u_audio;
uniform float u_openness;
uniform float u_width;
uniform vec4  u_state_weights; // x: listening, y: speaking, z: thinking, w: sleeping
uniform vec2  u_mouse;
uniform float u_muted;
uniform float u_pulse;

// --- Math helpers ---

mat2 rot2(float a) {
    float s = sin(a);
    float c = cos(a);
    return mat2(c, -s, s, c);
}

float hash21(vec2 p) {
    p = fract(p * vec2(234.34, 435.345));
    p += dot(p, p + 34.23);
    return fract(p.x * p.y);
}

float gyroid(vec3 p) {
    return dot(sin(p), cos(p.yzx));
}

// Smooth organic 3D displacement field (Lipschitz-controlled for clean raymarching)
float liquidField(vec3 p, float t, float speed, float voiceAmp, float openAmp) {
    vec3 q1 = p;
    q1.xz = rot2(t * 0.28 * speed) * q1.xz;
    q1.xy = rot2(t * 0.17 * speed) * q1.xy;

    vec3 q2 = p;
    q2.yz = rot2(-t * 0.42 * speed) * q2.yz;
    q2.xz = rot2(t * 0.31 * speed) * q2.xz;

    float g1 = gyroid(q1 * 2.35 + vec3(0.0, t * 0.48 * speed, t * 0.32 * speed));
    float g2 = gyroid(q2 * 4.55 - vec3(t * 0.72 * speed, 0.0, t * 0.54 * speed));
    float g3 = gyroid(q1 * 7.80 + vec3(t * 1.15, -t * 0.92, t * 0.65));

    float thinkBoost = u_state_weights.z;
    float sleepCalm  = u_state_weights.w;
    float muteCalm   = u_muted;

    float calmFactor = 1.0 - 0.55 * sleepCalm - 0.35 * muteCalm;

    float a1 = (0.048 + 0.032 * thinkBoost + voiceAmp * 0.068 + u_pulse * 0.055) * calmFactor;
    float a2 = (0.015 + 0.018 * thinkBoost + voiceAmp * 0.044 + openAmp * 0.032) * calmFactor;
    float a3 = (0.004 + voiceAmp * 0.014 + openAmp * 0.010) * calmFactor;

    return g1 * a1 + g2 * a2 + g3 * a3;
}

float mapOrb(vec3 p, float t, float speed, float voiceAmp, float openAmp) {
    // Subtle speech-width elastic morphing
    vec3 q = p;
    float stretch = u_width * (0.035 + 0.055 * voiceAmp);
    q.x *= 1.0 - stretch;
    q.y *= 1.0 + stretch * 0.65;

    float breath = sin(t * 1.45) * 0.022 + sin(t * 0.63 + 1.2) * 0.010;
    float baseR  = 0.86 + breath + voiceAmp * 0.075 + openAmp * 0.040 + u_pulse * 0.05;
    float disp   = liquidField(q, t, speed, voiceAmp, openAmp);

    return length(q) - baseR - disp;
}

vec3 calcNormal(vec3 p, float t, float speed, float voiceAmp, float openAmp) {
    vec2 e = vec2(0.0022, -0.0022);
    return normalize(
        e.xyy * mapOrb(p + e.xyy, t, speed, voiceAmp, openAmp) +
        e.yyx * mapOrb(p + e.yyx, t, speed, voiceAmp, openAmp) +
        e.yxy * mapOrb(p + e.yxy, t, speed, voiceAmp, openAmp) +
        e.xxx * mapOrb(p + e.xxx, t, speed, voiceAmp, openAmp)
    );
}

// Iridescent thin-film cosine palette
vec3 iridescence(float x, vec3 primary, vec3 secondary, vec3 accent) {
    vec3 phase = vec3(0.0, 0.28, 0.58);
    vec3 raw = 0.5 + 0.5 * cos(6.28318 * (x + phase));
    vec3 tinted = primary * raw.r + secondary * raw.g + accent * raw.b;
    return mix(raw, tinted, 0.78);
}

// Subtle procedural cosmic dust motes (proper 2D cell hash, zero banding/stripes)
float starLayer(vec2 uv, float scale, float t, float seedOffset) {
    vec2 gv = uv * scale;
    vec2 id = floor(gv);
    vec2 f  = fract(gv) - 0.5;

    float h = hash21(id + seedOffset);
    if (h < 0.78) return 0.0;

    vec2 jitter = vec2(
        hash21(id * 1.37 + 19.1 + seedOffset) - 0.5,
        hash21(id * 2.13 + 73.7 + seedOffset) - 0.5
    ) * 0.72;

    float d = length(f - jitter);
    float radius = mix(0.018, 0.055, fract(h * 17.3));
    float twinkle = 0.45 + 0.55 * sin(t * (0.9 + fract(h * 41.7) * 1.6) + h * 62.83);
    float core = smoothstep(radius, radius * 0.12, d);
    float halo = exp(-d * 18.0) * 0.22;
    return (core + halo) * twinkle * smoothstep(0.78, 0.99, h);
}

// Filmic ACES tonemap
vec3 aces(vec3 x) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

void main() {
    vec2 res = max(u_resolution, vec2(1.0));
    vec2 uv  = (gl_FragCoord.xy - u_viewport_origin) / res;
    float minDim = min(res.x, res.y);
    vec2 p = (uv - u_orb_center) * res / (minDim * max(u_orb_scale, 0.1));

    float t = u_time;
    vec4 w = u_state_weights;

    // --- Curated Obsidian + Violet-Cyan Neon State Palettes (Linear RGB) ---
    vec3 pListen = vec3(0.42, 0.18, 0.98);
    vec3 sListen = vec3(0.06, 0.84, 0.98);
    vec3 aListen = vec3(0.84, 0.24, 0.90);

    vec3 pSpeak  = vec3(0.04, 0.82, 0.96);
    vec3 sSpeak  = vec3(0.14, 0.98, 0.74);
    vec3 aSpeak  = vec3(0.54, 0.28, 1.00);

    vec3 pThink  = vec3(0.64, 0.16, 0.96);
    vec3 sThink  = vec3(1.00, 0.52, 0.16);
    vec3 aThink  = vec3(0.98, 0.22, 0.62);

    vec3 pSleep  = vec3(0.20, 0.14, 0.48);
    vec3 sSleep  = vec3(0.12, 0.36, 0.56);
    vec3 aSleep  = vec3(0.36, 0.24, 0.62);

    vec3 colPrimary   = pListen * w.x + pSpeak * w.y + pThink * w.z + pSleep * w.w;
    vec3 colSecondary = sListen * w.x + sSpeak * w.y + sThink * w.z + sSleep * w.w;
    vec3 colAccent    = aListen * w.x + aSpeak * w.y + aThink * w.z + aSleep * w.w;

    // Subtle crimson-rose shift when mic is muted
    vec3 muteTint = vec3(0.82, 0.14, 0.34);
    colPrimary   = mix(colPrimary,   mix(colPrimary, muteTint, 0.58), u_muted);
    colSecondary = mix(colSecondary, vec3(0.42, 0.22, 0.52), u_muted * 0.65);

    float speed = 0.85 * w.x + 1.35 * w.y + 1.75 * w.z + 0.38 * w.w;
    speed = mix(speed, 0.45, u_muted * 0.6);

    float voiceAmp = clamp(u_audio * (1.0 - 0.7 * u_muted), 0.0, 1.0);
    float openAmp  = clamp(u_openness * (1.0 - 0.7 * u_muted), 0.0, 1.0);

    // --- 1. Deep Obsidian Atmospheric Backdrop ---
    float radial = length(p);
    vec3 bg = mix(
        vec3(0.016, 0.012, 0.036),
        vec3(0.005, 0.004, 0.014),
        smoothstep(0.0, 2.35, radial)
    );

    // Slow volumetric aurora lobes behind the orb
    vec2 lobe1 = p - vec2(-0.55 * cos(t * 0.18),  0.42 * sin(t * 0.15));
    vec2 lobe2 = p - vec2( 0.62 * sin(t * 0.14), -0.38 * cos(t * 0.19));
    bg += colPrimary   * exp(-dot(lobe1, lobe1) * 0.52) * (0.055 + 0.035 * voiceAmp);
    bg += colSecondary * exp(-dot(lobe2, lobe2) * 0.62) * (0.042 + 0.035 * voiceAmp);

    // Subtle radial horizon mist beneath the orb (gives grounded studio-floor depth)
    vec2 floorPos = (p - vec2(0.0, -1.18)) * vec2(0.55, 2.4);
    bg += mix(colPrimary, colSecondary, 0.5) * exp(-dot(floorPos, floorPos) * 1.25) * (0.055 + 0.04 * voiceAmp);

    // Subtle 2-layer parallax cosmic dust
    vec2 aspectUv = (uv - 0.5) * vec2(res.x / minDim, res.y / minDim);
    float s1 = starLayer(aspectUv + vec2(t * 0.004, -t * 0.009) - u_mouse * 0.015, 7.5,  t, 11.0);
    float s2 = starLayer(aspectUv + vec2(-t * 0.003, -t * 0.005) - u_mouse * 0.007, 13.0, t, 47.0);
    float starMask = smoothstep(0.55, 1.45, radial);
    bg += mix(vec3(0.76, 0.80, 1.0), colSecondary, 0.35) * (s1 * 0.34 + s2 * 0.22) * starMask;

    // --- 2. 3D Camera & Mouse Parallax ---
    vec3 ro = vec3(0.0, 0.0, 3.15);
    vec3 rd = normalize(vec3(p, -2.05));

    float yaw   = -u_mouse.x * 0.24;
    float pitch =  u_mouse.y * 0.18;
    ro.yz = rot2(pitch) * ro.yz;
    rd.yz = rot2(pitch) * rd.yz;
    ro.xz = rot2(yaw)   * ro.xz;
    rd.xz = rot2(yaw)   * rd.xz;

    // --- 3. 3D Tilted Energy Orbital Ring (Ray-Plane Intersection) ---
    vec3 ringN = normalize(vec3(
        0.26 * sin(t * 0.22),
        0.89,
        0.38 * cos(t * 0.19) + 0.12
    ));
    float ringDenom = dot(rd, ringN);
    float tRing = -1.0;
    vec3 ringCol = vec3(0.0);
    if (abs(ringDenom) > 0.001) {
        float tr = -dot(ro, ringN) / ringDenom;
        if (tr > 0.2 && tr < 8.0) {
            vec3 rp = ro + rd * tr;
            float rDist = length(rp);
            float targetR = 1.24 + 0.04 * sin(t * 1.1) + voiceAmp * 0.08 + u_pulse * 0.12;
            float band = abs(rDist - targetR);
            float angle = atan(rp.z, rp.x);
            float wave = 0.55 + 0.45 * sin(angle * 3.0 - t * (1.2 + w.z * 1.8));
            float sparkle = pow(max(0.5 + 0.5 * sin(angle * 2.0 + t * 2.2), 0.0), 6.0);
            float filament = exp(-band * 62.0) * (0.42 + 0.45 * wave + 0.55 * sparkle)
                           + exp(-band * 16.0) * 0.10;
            float alphaRing = filament * (0.48 + 0.35 * voiceAmp + 0.35 * w.z) * (1.0 - 0.55 * w.w);
            if (alphaRing > 0.002) {
                tRing = tr;
                vec3 rc = mix(colPrimary, colSecondary, 0.5 + 0.5 * sin(angle - t * 0.8));
                ringCol = rc * alphaRing;
            }
        }
    }

    // --- 4. Raymarch the 3D Liquid Glass / Ferrofluid Orb ---
    float b = dot(ro, rd);
    float cBound = dot(ro, ro) - 1.48 * 1.48;
    float disc = b * b - cBound;

    bool hit = false;
    float tHit = 100.0;
    float minDist = 10.0;

    if (disc > 0.0) {
        float sDisc = sqrt(disc);
        float tNear = max(0.0, -b - sDisc);
        float tFar  = -b + sDisc;
        float tRay  = tNear;

        for (int i = 0; i < 64; i++) {
            vec3 pos = ro + rd * tRay;
            float d = mapOrb(pos, t, speed, voiceAmp, openAmp);
            minDist = min(minDist, max(d, 0.0));
            if (d < 0.0014) {
                hit = true;
                tHit = tRay;
                break;
            }
            tRay += d * 0.58;
            if (tRay > tFar) break;
        }
    }

    // Contoured outer atmospheric bloom hugging the live orb silhouette
    vec3 col = bg;
    float haloDist = hit ? 0.0 : minDist;
    float softBloom = exp(-radial * 1.75) * (0.20 + 0.16 * voiceAmp + 0.12 * u_pulse);
    float contourHalo = exp(-haloDist * 7.5) * (0.28 + 0.24 * voiceAmp + 0.18 * u_pulse);
    vec3 haloColor = mix(colPrimary, colSecondary, 0.45 + 0.35 * sin(atan(p.y, p.x) * 2.0 - t * 0.7));
    col += haloColor * (softBloom + contourHalo) * (1.0 - 0.45 * w.w);

    // Back half of 3D orbital ring (behind orb)
    if (tRing > 0.0 && (!hit || tRing > tHit)) {
        col += ringCol * 0.85;
    }

    // --- 5. Shade the 3D Liquid Glass / Ferrofluid Surface ---
    if (hit) {
        vec3 pos = ro + rd * tHit;
        vec3 N   = calcNormal(pos, t, speed, voiceAmp, openAmp);
        vec3 V   = -rd;
        float ndv = clamp(dot(N, V), 0.0, 1.0);

        // Refracted internal swirling plasma core (gives true crystal/liquid glass depth)
        vec3 refr = refract(rd, N, 1.0 / 1.38);
        vec3 innerAccum = vec3(0.0);

        for (int j = 0; j < 3; j++) {
            float depth = 0.28 + float(j) * 0.36;
            vec3 ip = pos + refr * depth;
            vec3 iq = ip;
            iq.xz = rot2(t * (0.35 + 0.2 * float(j)) * speed) * iq.xz;
            iq.xy = rot2(-t * 0.24 * speed + float(j) * 1.1) * iq.xy;

            float gIn = gyroid(iq * (2.6 + float(j) * 0.85) + vec3(0.0, t * 0.6 * speed, -t * 0.4));
            float ribbon = pow(clamp(1.0 - abs(gIn) * 0.78, 0.0, 1.0), 3.2);
            float coreFade = smoothstep(1.25, 0.15, length(ip));

            vec3 layerCol = (j == 0) ? colPrimary : ((j == 1) ? colSecondary : colAccent);
            innerAccum += layerCol * ribbon * coreFade * (0.58 + 0.55 * voiceAmp + 0.30 * openAmp);
        }

        // Deep obsidian-glass interior absorption
        vec3 deepCore = mix(vec3(0.015, 0.010, 0.045), colPrimary * 0.22, pow(ndv, 1.6));
        // Warm luminous heart right in the center
        float centerHeart = pow(ndv, 2.6) * (0.28 + 0.32 * voiceAmp + 0.18 * u_pulse);
        vec3 interior = deepCore + innerAccum + mix(colPrimary, colSecondary, 0.65) * centerHeart;

        // Dichroic thin-film iridescence on curved liquid surface
        float surfWave = gyroid(pos * 3.1 + vec3(t * 0.4, -t * 0.3, 0.0)) * 0.22;
        vec3 irid = iridescence(ndv * 1.05 + surfWave + t * 0.06, colPrimary, colSecondary, colAccent);

        // Fresnel glass reflection + subtle optical chromatic dispersion at rim
        float fresnel  = pow(1.0 - ndv, 2.8);
        float rimSharp = pow(1.0 - ndv, 5.2);
        vec3 fresnelDisp = vec3(
            pow(1.0 - ndv, 2.55),
            pow(1.0 - ndv, 2.85),
            pow(1.0 - ndv, 3.20)
        );

        // 3-point studio environment lighting (makes the 3D form pop like glossy ferrofluid)
        vec3 R = reflect(rd, N);

        // Key studio softbox (top-left front)
        vec3 L1 = normalize(vec3(-0.58, 0.72, 0.56));
        float specKeySharp = pow(max(dot(R, L1), 0.0), 64.0);
        float specKeyBroad = pow(max(dot(R, L1), 0.0), 10.0);
        float diffKey      = max(dot(N, L1), 0.0);

        // Electric fill light (bottom-right)
        vec3 L2 = normalize(vec3(0.74, -0.52, 0.34));
        float specFill = pow(max(dot(R, L2), 0.0), 18.0);
        float diffFill = max(dot(N, L2), 0.0);

        // Top zenith cyan edge sheen
        vec3 L3 = normalize(vec3(0.12, 0.94, -0.26));
        float specTop = pow(max(dot(R, L3), 0.0), 28.0);

        // Compose glossy liquid glass surface
        vec3 orbCol = interior * (0.72 + 0.35 * diffKey);
        orbCol += irid * fresnel * (0.85 + 0.35 * voiceAmp);
        orbCol += mix(colSecondary, vec3(0.92, 0.96, 1.0), 0.55) * rimSharp * (1.25 + 0.55 * voiceAmp);
        orbCol += fresnelDisp * colAccent * 0.28;

        // Crisp specular studio highlights (ferrofluid liquid chrome sheen)
        orbCol += vec3(0.96, 0.98, 1.00) * specKeySharp * 1.45;
        orbCol += mix(colSecondary, vec3(0.85, 0.90, 1.0), 0.5) * specKeyBroad * 0.34;
        orbCol += mix(colPrimary, colAccent, 0.5) * (specFill * 0.58 + diffFill * 0.14);
        orbCol += colSecondary * specTop * 0.65;

        // Micro-ripple sheen along high-curvature crests when speaking
        float crest = smoothstep(0.35, 1.0, abs(surfWave) * 3.8) * (0.18 + 0.55 * voiceAmp);
        orbCol += mix(colSecondary, vec3(1.0), 0.4) * crest * (0.35 + 0.65 * fresnel);

        // Smooth antialiased silhouette edge blend into outer glow
        float edgeAA = smoothstep(0.0, 0.12, ndv);
        col = mix(col + haloColor * 0.42, orbCol, edgeAA);
    }

    // Front half of 3D orbital ring (passes IN FRONT of the orb!)
    if (tRing > 0.0 && hit && tRing < tHit) {
        col += ringCol * 1.15;
    }

    // Filmic ACES tonemapping + linear-to-sRGB gamma
    col = aces(col * 1.12);
    col = pow(max(col, vec3(0.0)), vec3(1.0 / 2.2));

    // Subtle dithering to completely eliminate 8-bit dark gradient banding
    float dither = (hash21(gl_FragCoord.xy) - 0.5) / 255.0;
    col = clamp(col + dither, 0.0, 1.0);

    fragColor = vec4(col, 1.0);
}
"#;

impl OrbRenderer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let glsl_ver = gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION);
            let is_es = glsl_ver.contains(" ES ");
            let header = if is_es {
                "#version 300 es\nprecision highp float;\n"
            } else {
                "#version 140\n"
            };

            let vert_src = format!("{header}{VERT_BODY}");
            let frag_src = format!("{header}{FRAG_BODY}");

            let program = gl.create_program().map_err(|e| format!("create_program: {e}"))?;

            let vs = compile_shader(gl, glow::VERTEX_SHADER, &vert_src)?;
            let fs = match compile_shader(gl, glow::FRAGMENT_SHADER, &frag_src) {
                Ok(s) => s,
                Err(e) => {
                    gl.delete_shader(vs);
                    gl.delete_program(program);
                    return Err(e);
                }
            };

            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.link_program(program);

            let linked = gl.get_program_link_status(program);
            let log = gl.get_program_info_log(program);

            gl.detach_shader(program, vs);
            gl.detach_shader(program, fs);
            gl.delete_shader(vs);
            gl.delete_shader(fs);

            if !linked {
                gl.delete_program(program);
                return Err(format!("shader link error: {log}"));
            }

            let vao = gl
                .create_vertex_array()
                .map_err(|e| format!("create_vertex_array: {e}"))?;

            let u_viewport_origin = gl.get_uniform_location(program, "u_viewport_origin");
            let u_resolution = gl.get_uniform_location(program, "u_resolution");
            let u_time = gl.get_uniform_location(program, "u_time");
            let u_orb_center = gl.get_uniform_location(program, "u_orb_center");
            let u_orb_scale = gl.get_uniform_location(program, "u_orb_scale");
            let u_audio = gl.get_uniform_location(program, "u_audio");
            let u_openness = gl.get_uniform_location(program, "u_openness");
            let u_width = gl.get_uniform_location(program, "u_width");
            let u_state_weights = gl.get_uniform_location(program, "u_state_weights");
            let u_mouse = gl.get_uniform_location(program, "u_mouse");
            let u_muted = gl.get_uniform_location(program, "u_muted");
            let u_pulse = gl.get_uniform_location(program, "u_pulse");

            Ok(Self {
                program,
                vao,
                u_viewport_origin,
                u_resolution,
                u_time,
                u_orb_center,
                u_orb_scale,
                u_audio,
                u_openness,
                u_width,
                u_state_weights,
                u_mouse,
                u_muted,
                u_pulse,
            })
        }
    }

    pub fn paint(
        &self,
        gl: &glow::Context,
        viewport_origin: [f32; 2],
        resolution: [f32; 2],
        u: &OrbUniforms,
    ) {
        unsafe {
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::BLEND);

            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));

            gl.uniform_2_f32(
                self.u_viewport_origin.as_ref(),
                viewport_origin[0],
                viewport_origin[1],
            );
            gl.uniform_2_f32(self.u_resolution.as_ref(), resolution[0], resolution[1]);
            gl.uniform_1_f32(self.u_time.as_ref(), u.time);
            gl.uniform_2_f32(
                self.u_orb_center.as_ref(),
                u.orb_center[0],
                u.orb_center[1],
            );
            gl.uniform_1_f32(self.u_orb_scale.as_ref(), u.orb_scale);
            gl.uniform_1_f32(self.u_audio.as_ref(), u.audio);
            gl.uniform_1_f32(self.u_openness.as_ref(), u.openness);
            gl.uniform_1_f32(self.u_width.as_ref(), u.width);
            gl.uniform_4_f32(
                self.u_state_weights.as_ref(),
                u.state_weights[0],
                u.state_weights[1],
                u.state_weights[2],
                u.state_weights[3],
            );
            gl.uniform_2_f32(self.u_mouse.as_ref(), u.mouse[0], u.mouse[1]);
            gl.uniform_1_f32(self.u_muted.as_ref(), u.muted);
            gl.uniform_1_f32(self.u_pulse.as_ref(), u.pulse);

            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
        }
    }
}

unsafe fn compile_shader(
    gl: &glow::Context,
    shader_type: u32,
    source: &str,
) -> Result<glow::Shader, String> {
    unsafe {
        let shader = gl
            .create_shader(shader_type)
            .map_err(|e| format!("create_shader: {e}"))?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            return Err(format!("shader compile error: {log}"));
        }
        Ok(shader)
    }
}
