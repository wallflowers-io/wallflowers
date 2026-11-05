//  LaunchMark.swift
//  The launch sequence — the ORIGINAL Orbit-CMY, once.
//
//  A full black screen; the disc rests due NORTH of centre — horizontally centred — then
//  runs ONE Orbit-CMY revolution — a slow wind-up, a whip through the middle, an
//  ease-out — with three CMY circles (cyan · magenta · yellow) trailing by stepped lags
//  and blooming with their angular velocity. No squircle frame — the disc orbits on
//  plain black. It settles back to that same horizontally-centred home (clicks in), then
//  the gate fades to reveal the app.
//
//  Ported verbatim from animations/native/OrbitCMY (MetalOrbitView) — additive into an
//  extended-linear P3 target, so overlaps + bloom exceed 1.0 = real HDR on an EDR OLED
//  (clamps to white on the Simulator, which reports no headroom). Geometry + spin
//  easing from the orbit animation study.

import SwiftUI
import Metal
import QuartzCore

// MARK: - Shader

private let launchShaderSource = """
#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float time, resX, resY;
    float exposure, glowMax, discAlpha, glowGain, spin, lag, markScale, settleHold, bloomDur;
};
struct VOut { float4 pos [[position]]; float2 uv; };

vertex VOut v_main(uint vid [[vertex_id]]) {
    float2 p[3] = { float2(-1, -1), float2(3, -1), float2(-1, 3) };
    VOut o;
    o.pos = float4(p[vid], 0, 1);
    o.uv  = p[vid] * 0.5 + 0.5;
    return o;
}

// Spin easing tuned in the Motion Studio: cubic-bezier(0.82,-0.06,0.35,0.98)
// (slow wind-up, whip, ease-out). PEAK = its max |dy/dx|, so glow normalizes to 1.
constant float BX1 = 0.82, BY1 = -0.06, BX2 = 0.35, BY2 = 0.98;
constant float PEAK = 3.9296;
static inline float bezX(float t){ float cx=3.0*BX1, bx=3.0*(BX2-BX1)-cx, ax=1.0-cx-bx; return ((ax*t+bx)*t+cx)*t; }
static inline float bezY(float t){ float cy=3.0*BY1, by=3.0*(BY2-BY1)-cy, ay=1.0-cy-by; return ((ay*t+by)*t+cy)*t; }
static inline float bezDX(float t){ float cx=3.0*BX1, bx=3.0*(BX2-BX1)-cx, ax=1.0-cx-bx; return (3.0*ax*t+2.0*bx)*t+cx; }
static inline float easeCB(float x) {
    if (x <= 0.0) return 0.0;
    if (x >= 1.0) return 1.0;
    float t = x;
    for (int i = 0; i < 8; i++) { float xx = bezX(t) - x; if (fabs(xx) < 1e-5) break; float d = bezDX(t); if (fabs(d) < 1e-6) break; t -= xx / d; }
    return bezY(t);
}
static inline float speedNorm(float local) {
    float h = 0.004;
    float d = (easeCB(min(1.0, local + h)) - easeCB(max(0.0, local - h))) / (2.0 * h);
    return clamp(fabs(d) / PEAK, 0.0, 1.0);
}

fragment float4 f_main(VOut in [[stage_in]], constant Uniforms& u [[buffer(0)]]) {
    const float S = 100.0, MID = 50.0, RAD = 18.51;
    const float R = MID - RAD - 0.5;
    // Rest due NORTH: x = MID, so the disc starts AND finishes HORIZONTALLY centred,
    // sitting above centre, rather than in the icon's NW quadrant. (y-down space, so
    // −π/2 points up.) One revolution returns to the same point.
    const float HOME = -1.57079632679;
    const float TAU = 6.28318530718;

    float2 res = float2(u.resX, u.resY);
    float m = min(res.x, res.y);
    float2 origin = (res - m) * 0.5;
    float2 frag = float2(in.uv.x, 1.0 - in.uv.y) * res;
    float2 pt = (frag - origin) / m * S;
    // Full-screen canvas: S=100 maps to the short screen side; markScale shrinks the
    // whole mark about centre (0.527 = the old 0.62 square, 15% smaller).
    pt = (pt - MID) / u.markScale + MID;

    const float3 COL[4] = { float3(1,1,1), float3(0,1,1), float3(1,0,1), float3(1,1,0) };
    float lags[4] = { 0.0, u.lag, 2.0 * u.lag, 3.0 * u.lag };

    // ONE-SHOT: no fmod — after the spin the disc settles at HOME and holds.
    float tau = u.time;

    // THE BLOOM is a GLOW, not a growing shape: the same gaussian the tap press-bloom uses
    // (Shell/Bloom.metal `discBloom` — exp(-d²/2σ²) with σ swelling), just scaled to the
    // screen instead of a 64pt disc. Growing a hard-edged disc reads as an inflating circle;
    // growing σ reads as light swelling out of the mark, which is the intent.
    //
    // It starts ramping as the mark comes through due W — 270° of the revolution measured
    // from N and travelling clockwise (N→E→S→W→N), so 270/360 = 0.75 of the way round. The
    // full final quarter is therefore a dissolve from hard disc into pure light, which is
    // what keeps a stationary hard-edged circle from ever appearing at rest.
    const float W = 0.75;
    float p0 = easeCB(clamp(tau / u.spin, 0.0, 1.0));   // the white disc's eased progress
    float arrive = smoothstep(W, 1.0, p0);              // 0 at W → 1 as it lands at N

    // Once home, the glow takes over and swells to whiteout.
    float bl = clamp((tau - (u.spin + u.settleHold)) / max(u.bloomDur, 1e-4), 0.0, 1.0);
    float bloom = bl * bl * (3.0 - 2.0 * bl);           // smoothstep ease

    // σ must reach the farthest CORNER, computed from the resolution rather than assumed:
    // a spread tuned to the short side leaves the corners grey on a tall screen.
    float2 corner = ((res - origin) / m * S - MID) / u.markScale + MID;
    float maxR = distance(corner, float2(MID, MID)) + 1.0;

    float3 col = float3(0.0);
    for (int i = 0; i < 4; i++) {
        float local = clamp((tau - lags[i]) / u.spin, 0.0, 1.0);
        float ang = HOME + easeCB(local) * TAU;
        float2 c = float2(MID + R * cos(ang), MID + R * sin(ang));
        float d = distance(pt, c);
        float vel = speedNorm(local);
        float disc = 1.0 - smoothstep(RAD - 0.7, RAD + 0.7, d);
        float sigma = max(u.glowMax * vel, 0.001);
        float halo = exp(-pow(max(0.0, d - RAD), 2.0) / (2.0 * sigma * sigma));
        // The orbit dissolves INTO the glow across the WNW→home approach, so the mark is
        // already pure light by the time it comes to rest. Keyed to `arrive`, NOT `bloom`:
        // all four discs finish a full revolution at the SAME point, so anything still hard
        // at rest sums to a stationary hard-edged white circle before the bloom begins.
        col += COL[i] * (disc * u.discAlpha + halo * u.glowGain * vel) * (1.0 - arrive);
    }

    // The glow itself — centred on HOME, where the mark comes to rest.
    float2 home = float2(MID + R * cos(HOME), MID + R * sin(HOME));
    float dh = distance(pt, home);
    float gsig = mix(RAD * 0.85, maxR * 1.15, bloom);
    float g = exp(-(dh * dh) / (2.0 * gsig * gsig));
    // Overdrive at full bloom so the falloff still clips to flat white at the edges — the
    // gate then cross-fades a clean white, not a vignette.
    col += float3(1.0) * g * arrive * mix(1.0, 3.4, bloom);

    // No frame — the disc orbits on full black.
    col *= u.exposure;
    return float4(col, 1.0);
}
"""

/// Uniforms — field order/types must match `struct Uniforms` in the shader.
struct LaunchUniforms {
    var time: Float = 0
    var resX: Float = 0
    var resY: Float = 0
    var exposure: Float = 1.4
    var glowMax: Float = 8.0
    var discAlpha: Float = 0.6
    var glowGain: Float = 1.8
    var spin: Float = 0.76
    var lag: Float = 0.026
    var markScale: Float = 0.527   // 0.62 (old square) × 0.85 = 15% smaller, full-screen
    /// No beat at HOME. The glow now ramps across the whole final quarter (from due W), so by
    /// the time the mark lands there is no disc left to "click in" — a hold here would just be
    /// a stall on a static glow before the fill starts.
    var settleHold: Float = 0.0
    /// How long the glow takes to swell from the mark to a screen-filling white. Shorter than
    /// the WNW version: the glow arrives further developed, so it has less distance to cover.
    var bloomDur: Float = 0.24

    /// Seconds until the disc has settled at HOME (one revolution complete).
    var motionDuration: Double { Double(spin + 3 * lag) }
    /// Seconds until the glow has filled the screen — the moment the gate should start fading,
    /// so the app is revealed from behind a flat white rather than mid-motion. MUST track the
    /// shader's `u.spin + u.settleHold` bloom start (the WHITE disc lands at `spin`; the lagged
    /// CMY trail dissolves into the glow rather than gating it).
    var fillDuration: Double { Double(spin + settleHold + bloomDur) }
}

// MARK: - Metal view

/// UIView backed by an EDR-configured CAMetalLayer.
final class LaunchMetalUIView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }
    private var metalLayer: CAMetalLayer { layer as! CAMetalLayer }

    private let device = MTLCreateSystemDefaultDevice()!
    private var queue: MTLCommandQueue!
    private var pipeline: MTLRenderPipelineState!
    private var link: CADisplayLink?
    private var start: CFTimeInterval = CACurrentMediaTime()

    var uniforms = LaunchUniforms()

    override init(frame: CGRect) { super.init(frame: frame); configure() }
    required init?(coder: NSCoder) { super.init(coder: coder); configure() }

    private func configure() {
        queue = device.makeCommandQueue()

        metalLayer.device = device
        metalLayer.pixelFormat = .rgba16Float
        metalLayer.colorspace = CGColorSpace(name: CGColorSpace.extendedLinearDisplayP3)
        metalLayer.isOpaque = true
        metalLayer.framebufferOnly = true

        let lib: MTLLibrary
        do {
            lib = try device.makeLibrary(source: launchShaderSource, options: nil)
        } catch {
            print("Launch shader compile failed: \\(error)")
            return
        }
        let desc = MTLRenderPipelineDescriptor()
        desc.vertexFunction = lib.makeFunction(name: "v_main")
        desc.fragmentFunction = lib.makeFunction(name: "f_main")
        desc.colorAttachments[0].pixelFormat = .rgba16Float
        pipeline = try? device.makeRenderPipelineState(descriptor: desc)

    }

    /// THE LINK LIVES ONLY WHILE THE MARK IS ON SCREEN.
    ///
    /// It used to be started in `configure()` and never stopped — no `invalidate`,
    /// no `deinit` — so a display link ran on the main runloop for the whole life of
    /// the process, long after the launch animation was gone. Two consequences, and
    /// the second is why this is being fixed now:
    ///
    ///  • it draws every frame forever (the measured main-thread `nextDrawable`
    ///    stall behind "unusably slow"), and
    ///  • THE APP NEVER GOES IDLE. XCUITest's every launch begins "Wait for
    ///    network.pacific to idle", and a live display link means that wait can
    ///    never be satisfied — so under any real load the runner dies there, before
    ///    a single tap. That is what killed the fleet's pairing steps whenever the
    ///    three cameras were rolling, and it is why the untouched FeedUITests suite
    ///    dies the same way.
    ///
    /// `deinit` alone would not do: CADisplayLink RETAINS its target, so the view
    /// could never deallocate while the link was alive — the cycle that kept it
    /// running is exactly the one that stops `deinit` ever being called. Leaving the
    /// window is the real signal, and it is reversible: come back and it restarts.
    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil { stopLink() } else { startLink() }
    }

    deinit { link?.invalidate() }

    private func startLink() {
        guard link == nil else { return }
        let dl = CADisplayLink(target: self, selector: #selector(tick))
        dl.add(to: .main, forMode: .common)
        link = dl
    }

    private func stopLink() {
        link?.invalidate()
        link = nil
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = window?.screen.scale ?? UIScreen.main.scale
        metalLayer.contentsScale = scale
        metalLayer.drawableSize = CGSize(width: bounds.width * scale, height: bounds.height * scale)
    }

    @objc private func tick() {
        guard let pipeline, let drawable = metalLayer.nextDrawable() else { return }

        var u = uniforms
        u.time = Float(CACurrentMediaTime() - start)
        u.resX = Float(metalLayer.drawableSize.width)
        u.resY = Float(metalLayer.drawableSize.height)

        let rp = MTLRenderPassDescriptor()
        rp.colorAttachments[0].texture = drawable.texture
        rp.colorAttachments[0].loadAction = .clear
        rp.colorAttachments[0].clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 1)
        rp.colorAttachments[0].storeAction = .store

        guard let cb = queue.makeCommandBuffer(),
              let enc = cb.makeRenderCommandEncoder(descriptor: rp) else { return }
        enc.setRenderPipelineState(pipeline)
        enc.setFragmentBytes(&u, length: MemoryLayout<LaunchUniforms>.stride, index: 0)
        enc.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
        enc.endEncoding()
        cb.present(drawable)
        cb.commit()
    }
}

/// SwiftUI wrapper.
struct WakeOrbitMark: UIViewRepresentable {
    func makeUIView(context: Context) -> LaunchMetalUIView {
        let v = LaunchMetalUIView()
        v.backgroundColor = .black
        return v
    }
    func updateUIView(_ v: LaunchMetalUIView, context: Context) {}
}

// MARK: - Launch gate

/// Full-screen black launch overlay: the Orbit-CMY mark, centred. Runs one revolution,
/// settles (clicks in), then calls `onFinish` so the gate can fade to the app.
struct LaunchView: View {
    var onFinish: () -> Void
    private let u = LaunchUniforms()

    /// How long the reveal takes — the aperture opening and the white fading, together. Pulled
    /// in with the earlier ramp so the reveal stays continuous with the fill rather than trailing.
    private let revealDur: Double = 0.55
    /// The fraction of the aperture that is fully clear before it feathers back to opaque. Wide
    /// (small value) so the edge is a long gradient, not a rim — the aperture should read as the
    /// white thinning out, not as a hole with a border.
    private let feather: CGFloat = 0.32

    /// 0 = gate intact, 1 = fully open. Drives BOTH the mask and the gate's own opacity.
    @State private var reveal: CGFloat = 0

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            WakeOrbitMark().ignoresSafeArea()
        }
        .mask(aperture)
        // The whole gate ALSO fades as it opens, so the white recedes AND dissolves at once. On
        // its own the aperture is a bare geometric hole (tacky); coincided with the fade it reads
        // as the white simply thinning away from the centre. The fade LEADS the aperture (pow
        // 0.55, so opacity drops fast early) — the white is mostly gone before the hole is wide,
        // so the reveal is carried by the dissolve and the aperture only shapes its direction.
        .opacity(1.0 - Double(pow(reveal, 0.55)))
        .ignoresSafeArea()
        .task {
            // Once the glow has filled the screen, OPEN + FADE rather than cross-fade. A uniform
            // fade takes the white to the app's dark slate everywhere at once, which reads as a
            // flash to black; a growing soft aperture that dims as it widens keeps the white
            // receding outwards, continuing the bloom's direction of travel.
            try? await Task.sleep(nanoseconds: UInt64(u.fillDuration * 1_000_000_000))
            withAnimation(.easeInOut(duration: revealDur)) { reveal = 1 }
            // Hand back only once the gate is fully transparent, so its removal is invisible.
            try? await Task.sleep(nanoseconds: UInt64(revealDur * 1_000_000_000))
            onFinish()
        }
    }

    /// The gate is KEPT where this mask is opaque, so a clear core growing from the centre eats
    /// the white outwards. Sized off the DIAGONAL so the aperture clears the corners — a radius
    /// tuned to the short side would strand white wedges top and bottom.
    private var aperture: some View {
        GeometryReader { geo in
            let maxR = max(hypot(geo.size.width, geo.size.height) * 0.5, 1)
            RadialGradient(
                gradient: Gradient(stops: [
                    .init(color: .clear, location: 0),
                    .init(color: .clear, location: feather),
                    .init(color: .white, location: 1),
                ]),
                center: .center,
                startRadius: 0,
                // Scaled by 1/feather so it is the CLEAR core — not the feathered tail — that
                // reaches maxR at reveal == 1; otherwise the corners never fully open.
                endRadius: max((maxR / feather) * reveal, 0.001)
            )
            .ignoresSafeArea()
        }
        .ignoresSafeArea()
    }
}

#Preview("Launch — Orbit-CMY once") {
    LaunchView(onFinish: {})
}
