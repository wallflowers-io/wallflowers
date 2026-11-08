#include <metal_stdlib>
using namespace metal;

// Radial bloom burst for a tab disc (NOTE·CHAT·LIFE). `progress` 0→1 drives a
// grow-and-fade envelope; the glow tints from c1 (core) to c2 (outer). For most
// discs c1 == c2; LIFE passes the profile colour → blue so its bloom carries
// hints of the avatar's colour as well as blue. Used via SwiftUI `.colorEffect`.
[[ stitchable ]]
half4 discBloom(float2 pos, half4 color, float2 size, float progress,
                half4 c1, half4 c2) {
    float2 center = size * 0.5;
    float radius = min(size.x, size.y) * 0.5;
    float d = distance(pos, center) / radius;          // 0 centre … 1 box edge

    float env = sin(progress * 3.14159265);            // 0 → 1 → 0 across the press
    float sigma = mix(0.16, 0.6, progress);            // soft glow spread grows
    float g = exp(-(d * d) / (2.0 * sigma * sigma));   // gaussian bloom, 1 at centre
    float a = clamp(g * env * 1.4, 0.0, 1.0);

    half4 col = mix(c1, c2, half(clamp(d / max(sigma * 1.6, 0.001), 0.0, 1.0)));
    return half4(col.rgb * half(a), half(a));            // premultiplied
}

// ─────────────────────────────────────────────────────────────────────────────
// The BOHR BLOOM — ported from the one-pager's `bohrGpu` field, playing behind a compose canvas to light the ground AROUND it.
//
// Faithful to the original where it matters: six lobes, each a tight core gaussian
// plus a wide halo at 0.34; a centre that opens with motion; a page-scale BLEED
// that carries light past the disc's own box; and premultiplied alpha taken as the
// PEAK CHANNEL so black ground stays black instead of washing grey.
//
// ── WHY THE RING IS AN ELLIPSE, AND WHY THAT IS THE WHOLE BUG FIX ────────────
// The first cut put the ring at a fixed R = 0.52 of the short side, which on a
// 393x852 phone is |z| = 0.52 — INSIDE the compose card's footprint. The card
// covers x 16..W-16, y 57..H-90, so the only ground you can actually see is a thin
// frame: a 57pt band at the top, ~90pt at the bottom, 16pt strips at the sides.
// In z space that frame sits at |z_y| ~ 1.0 and |z_x| ~ 0.48.
//
// So every vividly-moving feature — the six lobes AND the motion-driven centre —
// was hidden behind the canvas, and the ONLY term reaching the visible bands was
// the wide bleed, which carried no motion factor at all. A dead constant times a
// slowly-drifting hue. That is why it read as lighting with no movement: it WAS
// lighting with no movement.
//
// The ring is an ELLIPSE, and the caller passes its CENTRE and radii — because only
// the caller knows where the canvas is, and the canvas moves. The lobes ride BEHIND the card (Ralph, 18 Aug): the
// light glows out from under it and spills at the edges, rather than orbiting in
// the open where the ring reads as six moving blobs.
[[ stitchable ]]
half4 bohrBloom(float2 pos, half4 color, float2 size, float2 centre, float2 ring,
                float spin, float sweep, float hueCh, float motion,
                half4 h0, half4 h1, half4 h2) {
    // Centred ON THE CANVAS, not on the view. The caller passes the card's live centre
    // so the whole field travels with it — which is what keeps the bloom anchored while
    // the keyboard pushes the card up and lets it back down.
    //
    // Normalised to the SHORT side so the field stays circular in feel and the sigmas
    // keep the original's proportions.
    float unit = max(min(size.x, size.y), 1.0);
    float2 z = (pos - centre) / unit;

    const float CORE = 4.0 / 200.0;      // CORE_H — the lobes' own radius
    const float HALO = 26.0 / 200.0;     // the per-lobe wide sigma

    float3 col = float3(0.0);
    for (int i = 0; i < 6; ++i) {
        float off = float(i) * (M_PI_F / 3.0);
        float2 p = float2(cos(spin + off), sin(spin + off)) * ring;
        float d = distance(z, p);

        // Brightness sweeps the ring on its OWN period so one lobe leads — the
        // original's per-segment gain, which is what stops the ring reading as a
        // static halo. `sweep` beats against `spin`; both arrive pre-wrapped.
        float bg = 0.35 + 0.65 * (0.5 + 0.5 * cos(sweep + off));

        float c = exp(-(d * d) / (2.0 * CORE * CORE));
        float h = exp(-(d * d) / (2.0 * HALO * HALO)) * 0.34;

        // Three opal hues around the ring, lerped continuously — the site's `hue(ch)`.
        float ch = fmod(float(i) * 0.5 + hueCh, 3.0);
        int lo = int(ch);
        float f = ch - float(lo);
        float3 A = lo == 0 ? float3(h0.rgb) : (lo == 1 ? float3(h1.rgb) : float3(h2.rgb));
        float3 B = lo == 0 ? float3(h1.rgb) : (lo == 1 ? float3(h2.rgb) : float3(h0.rgb));
        col += mix(A, B, f) * bg * (c + h);
    }

    // The drifting opal the wide terms wear, taken off the same hue channel so it
    // needs no trig of its own.
    float3 opal = mix(float3(h0.rgb), float3(h2.rgb), clamp(hueCh / 3.0, 0.0, 1.0));

    // The centre, opening with total motion.
    float dc = length(z);
    float sc = (26.0 + 62.0 * motion) / 200.0;
    col += opal * motion * exp(-(dc * dc) / (2.0 * sc * sc));

    // THE BLEED — page-scale sigma, a fraction of the amplitude, and now BREATHING.
    // In the original this gain is a live uniform; holding it constant is what left
    // the visible bands motionless.
    const float BLEED = 620.0 / 200.0;
    col += opal * exp(-(dc * dc) / (2.0 * BLEED * BLEED)) * 0.42 * (0.35 + 0.65 * motion);

    float alpha = clamp(max(col.r, max(col.g, col.b)), 0.0, 1.0);
    return half4(half3(clamp(col, 0.0, alpha)), half(alpha));
}
