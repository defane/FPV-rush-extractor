/// What one sampled frame tells us. Every value here was chosen by measuring it
/// on real footage, and the thresholds that read them live in `DetectorSettings`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameMetrics {
    /// Mean brightness. Below `black_luma` there is no picture at all.
    pub luma: f64,
    /// Variance of the Laplacian over the whole frame. Diagnostic only: a global
    /// threshold on it does *not* separate snow from a picture, because detailed
    /// foliage scores as high as snow — just not as uniformly (CLAUDE.md 4.1).
    pub laplacian_variance: f64,
    /// Coefficient of variation of the per-block Laplacian variance. Snow spreads
    /// identical high-frequency energy over the whole frame; a real picture puts
    /// flat sky next to detailed trees. The strongest single discriminator.
    pub block_variation: f64,
    /// Vertical lag-1 pixel correlation. Composite video is band-limited
    /// horizontally, so snow keeps a fair horizontal correlation and loses the
    /// vertical one — which is why the horizontal one is useless here.
    pub vertical_correlation: f64,
    /// Fraction of blocks carrying no high-frequency energy at all. Analog video
    /// always has noise; the goggles' own screens are synthesised.
    pub flat_fraction: f64,
    /// Correlation with the previous sampled frame. Filled in by the sequence.
    pub scene_correlation: f64,
    /// Median `scene_correlation` over a window. Interference from a second
    /// transmitter tears the picture differently every frame, so it has no
    /// temporal continuity. Filled in by the sequence.
    pub coherence: f64,
    /// Positive noise evidence, populated only for Snow Split.
    pub snow_split: Option<crate::SnowSplitEvidence>,
}

impl FrameMetrics {
    /// The per-frame values; the sequence-level ones start at their defaults.
    pub fn new(luma: f64, laplacian_variance: f64, block_variation: f64,
               vertical_correlation: f64, flat_fraction: f64) -> Self {
        Self {
            luma, laplacian_variance, block_variation, vertical_correlation, flat_fraction,
            scene_correlation: 1.0, coherence: 1.0, snow_split: None,
        }
    }
}

/// Computes the per-frame metrics from an 8-bit grayscale buffer.
///
/// Plain portable code on purpose (CLAUDE.md 9.7): this is arithmetic, and the
/// release build already keeps pace with numpy.
#[derive(Debug, Clone, Copy)]
pub struct FrameAnalyzer {
    pub width: usize,
    pub height: usize,
}

impl Default for FrameAnalyzer {
    fn default() -> Self {
        Self { width: 640, height: 480 }
    }
}

impl FrameAnalyzer {
    /// The block grid the high-frequency uniformity is measured over.
    pub const BLOCK_ROWS: usize = 6;
    pub const BLOCK_COLUMNS: usize = 8;
    /// Per-block Laplacian variance under which a block carries no noise at all.
    /// At 20 blown-out sky reached 0.50 and real flight footage read as a goggle
    /// screen; at 2 the worst real frame is 0.33 (CLAUDE.md 10).
    pub const FLAT_EPSILON: f64 = 2.0;
    /// Working size of the frame used for the scene correlation.
    pub const SMALL_WIDTH: usize = 40;
    pub const SMALL_HEIGHT: usize = 30;

    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height }
    }

    /// A 16x16 box average, matching the Python reference exactly. A resampling
    /// filter with a support window would not be reproducible across the two.
    pub fn downsample(&self, pixels: &[u8]) -> Vec<f64> {
        let block_width = self.width / Self::SMALL_WIDTH;
        let block_height = self.height / Self::SMALL_HEIGHT;
        let count = (block_width * block_height) as f64;
        let mut small = vec![0.0; Self::SMALL_WIDTH * Self::SMALL_HEIGHT];
        for row in 0..Self::SMALL_HEIGHT {
            for column in 0..Self::SMALL_WIDTH {
                let mut sum = 0.0;
                for y in 0..block_height {
                    let base = (row * block_height + y) * self.width + column * block_width;
                    for &pixel in &pixels[base..base + block_width] {
                        sum += pixel as f64;
                    }
                }
                small[row * Self::SMALL_WIDTH + column] = sum / count;
            }
        }
        small
    }

    pub fn metrics(&self, pixels: &[u8]) -> FrameMetrics {
        let (width, height) = (self.width, self.height);
        assert_eq!(pixels.len(), width * height, "frame is not {width}x{height}");

        let mut luma_sum = 0.0;
        for &pixel in pixels {
            luma_sum += pixel as f64;
        }

        // 4-neighbour Laplacian over the interior, as the Python reference does.
        let (inner_width, inner_height) = (width - 2, height - 2);
        let mut laplacian = vec![0.0; inner_width * inner_height];
        for y in 1..height - 1 {
            let row = y * width;
            for x in 1..width - 1 {
                laplacian[(y - 1) * inner_width + (x - 1)] = -4.0 * pixels[row + x] as f64
                    + pixels[row - width + x] as f64 + pixels[row + width + x] as f64
                    + pixels[row + x - 1] as f64 + pixels[row + x + 1] as f64;
            }
        }

        let (variation, flat) = Self::block_variation(&laplacian, inner_width, inner_height);
        FrameMetrics::new(
            luma_sum / pixels.len() as f64,
            variance(laplacian.iter().copied()),
            variation,
            self.vertical_correlation(pixels),
            flat,
        )
    }

    /// (coefficient of variation of the per-block variance, fraction of flat blocks)
    fn block_variation(laplacian: &[f64], width: usize, height: usize) -> (f64, f64) {
        let block_height = height / Self::BLOCK_ROWS;
        let block_width = width / Self::BLOCK_COLUMNS;
        let mut variances = Vec::with_capacity(Self::BLOCK_ROWS * Self::BLOCK_COLUMNS);
        for row in 0..Self::BLOCK_ROWS {
            for column in 0..Self::BLOCK_COLUMNS {
                // Row-major within the block: the order parity was established in.
                let block = (0..block_height).flat_map(|y| {
                    let base = (row * block_height + y) * width + column * block_width;
                    laplacian[base..base + block_width].iter().copied()
                });
                variances.push(variance(block));
            }
        }
        let n = variances.len() as f64;
        let mean = variances.iter().fold(0.0, |sum, v| sum + v) / n;
        let deviation = (variances.iter().fold(0.0, |sum, v| sum + (v - mean) * (v - mean)) / n)
            .sqrt();
        let flat = variances.iter().filter(|&&v| v < Self::FLAT_EPSILON).count() as f64 / n;
        (deviation / (mean + 1e-6), flat)
    }

    fn vertical_correlation(&self, pixels: &[u8]) -> f64 {
        let count = (self.height - 1) * self.width;
        let upper: Vec<f64> = pixels[..count].iter().map(|&p| p as f64).collect();
        let lower: Vec<f64> = pixels[self.width..self.width + count].iter()
            .map(|&p| p as f64).collect();
        Self::correlation(&upper, &lower)
    }

    /// Pearson correlation, zero when either side is constant.
    pub fn correlation(a: &[f64], b: &[f64]) -> f64 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let n = a.len() as f64;
        let mean_a = a.iter().fold(0.0, |sum, v| sum + v) / n;
        let mean_b = b.iter().fold(0.0, |sum, v| sum + v) / n;
        let (mut product, mut squares_a, mut squares_b) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(b) {
            let (da, db) = (x - mean_a, y - mean_b);
            product += da * db;
            squares_a += da * da;
            squares_b += db * db;
        }
        let denominator = (squares_a * squares_b).sqrt();
        if denominator == 0.0 { 0.0 } else { product / denominator }
    }
}

/// Population variance, summed in iteration order: two passes, so the values
/// are walked twice and the iterator must be cloneable.
fn variance(values: impl Iterator<Item = f64> + Clone) -> f64 {
    let (count, sum) = values.clone().fold((0usize, 0.0), |(n, sum), v| (n + 1, sum + v));
    if count == 0 {
        return 0.0;
    }
    let mean = sum / count as f64;
    values.fold(0.0, |total, v| total + (v - mean) * (v - mean)) / count as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsamples_by_box_average_so_both_implementations_agree() {
        // A ramp: every 16x16 block averages to the mean of its 16 columns.
        let pixels: Vec<u8> = (0..640 * 480).map(|i| ((i % 640) % 16) as u8).collect();
        let small = FrameAnalyzer::default().downsample(&pixels);
        assert_eq!(small.len(), 40 * 30);
        assert!(small.iter().all(|v| (v - 7.5).abs() < 1e-9));
    }

    #[test]
    fn correlates_to_zero_against_a_constant() {
        assert_eq!(FrameAnalyzer::correlation(&[1.0, 2.0, 3.0], &[5.0, 5.0, 5.0]), 0.0);
        assert!((FrameAnalyzer::correlation(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]) - 1.0).abs() < 1e-12);
    }
}
