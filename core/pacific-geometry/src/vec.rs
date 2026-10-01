//! Shared vector helpers — the exact trio SpectralClustering exposes in Swift
//! (normalize / dot / sq_dist) plus cosine. f32 throughout (Swift `Float`).

/// Unit-normalize; a zero vector is returned unchanged (never NaN).
pub fn normalize(v: &[f32]) -> Vec<f32> {
    let n = dot(v, v).sqrt();
    if n <= 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / n).collect()
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn sq_dist(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    dot(a, b)
}
