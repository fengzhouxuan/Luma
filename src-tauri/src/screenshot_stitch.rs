use image::RgbaImage;
use std::collections::HashMap;

const BANDS: usize = 3;
type Row = [u64; BANDS];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Update {
    Unchanged,
    Appended,
    NoOverlap,
    Limit,
}

pub(crate) struct Stitcher {
    width: u32,
    height: u32,
    max_height: u32,
    last: Vec<Row>,
    pixels: Vec<u8>,
}

impl Stitcher {
    pub(crate) fn new(frame: RgbaImage, max_height: u32) -> Self {
        let last = fingerprint(&frame);
        Self {
            width: frame.width(),
            height: frame.height(),
            max_height,
            last,
            pixels: frame.into_raw(),
        }
    }

    pub(crate) fn height(&self) -> u32 {
        (self.pixels.len() / (self.width as usize * 4)) as u32
    }

    pub(crate) fn append(&mut self, frame: RgbaImage) -> Update {
        if frame.dimensions() != (self.width, self.height) {
            return Update::NoOverlap;
        }
        let next = fingerprint(&frame);
        if self.last == next {
            return Update::Unchanged;
        }
        let Some((shift, footer)) = align(&self.last, &next) else {
            // Caret blink and small hover effects can repaint part of an
            // otherwise stationary viewport. Keep its last accepted pixels.
            let changed: usize = self
                .last
                .iter()
                .zip(&next)
                .map(|(before, after)| {
                    (0..BANDS)
                        .filter(|&band| before[band] != after[band])
                        .count()
                })
                .sum();
            return if changed * 25 <= next.len() * BANDS {
                Update::Unchanged
            } else {
                Update::NoOverlap
            };
        };
        if self.height() + shift as u32 > self.max_height {
            return Update::Limit;
        }
        let stride = self.width as usize * 4;
        self.pixels.truncate(self.pixels.len() - footer * stride);
        let start = (self.height as usize - footer - shift) * stride;
        self.pixels.extend_from_slice(&frame.as_raw()[start..]);
        self.last = next;
        Update::Appended
    }

    pub(crate) fn into_image(self) -> RgbaImage {
        RgbaImage::from_raw(self.width, self.height(), self.pixels)
            .expect("stitched image contains complete rows")
    }
}

fn fingerprint(image: &RgbaImage) -> Vec<Row> {
    let width = image.width() as usize;
    let margin = (width / 16).clamp(2, 22);
    let inner = width - margin * 2;
    image
        .as_raw()
        .chunks_exact(width * 4)
        .map(|row| {
            std::array::from_fn(|band| {
                let start = margin + band * inner / BANDS;
                let end = margin + (band + 1) * inner / BANDS;
                let step = ((end - start) / 160).max(1);
                (start..end)
                    .step_by(step)
                    .fold(0xcbf29ce484222325u64, |hash, x| {
                        let pixel = &row[x * 4..][..3];
                        let color =
                            u32::from_le_bytes([pixel[0] & 248, pixel[1] & 248, pixel[2] & 248, 0]);
                        (hash ^ u64::from(color)).wrapping_mul(0x100000001b3)
                    })
            })
        })
        .collect()
}

fn align(previous: &[Row], next: &[Row]) -> Option<(usize, usize)> {
    let height = previous.len();
    let fixed = |index: usize| previous[index] == next[index];
    let top = (0..height).take_while(|&index| fixed(index)).count();
    let end = (top..height).rfind(|&index| !fixed(index))? + 1;
    let footer = height - end;
    let body = end - top;
    // Leave enough visible content to verify the seam, even when the viewport
    // contains a large fixed header or footer.
    let overlap = (body / 6).clamp(24, 96);
    let max_shift = body.checked_sub(overlap)?;
    if max_shift == 0 {
        return None;
    }
    let mut votes = vec![0u32; max_shift + 1];
    for band in 0..BANDS {
        let mut rows: HashMap<u64, Vec<usize>> = HashMap::new();
        for (index, row) in previous.iter().enumerate().take(end).skip(top) {
            let matches = rows.entry(row[band]).or_default();
            // Repeated blank rows and other common patterns cannot anchor a seam.
            if matches.len() <= 12 {
                matches.push(index);
            }
        }
        for y in top..end {
            if fixed(y) {
                continue;
            }
            let Some(indices) = rows.get(&next[y][band]).filter(|rows| rows.len() <= 12) else {
                continue;
            };
            for &old_y in indices {
                if let Some(shift) = old_y
                    .checked_sub(y)
                    .filter(|&shift| shift > 0 && shift <= max_shift)
                {
                    votes[shift] += 1;
                }
            }
        }
    }
    let mut candidates: Vec<_> = (1..=max_shift).filter(|&shift| votes[shift] >= 6).collect();
    candidates.sort_unstable_by_key(|&shift| std::cmp::Reverse(votes[shift]));
    let mut verified = candidates.into_iter().take(16).filter(|&shift| {
        let matched: usize = (top..end - shift)
            .map(|y| {
                (0..BANDS)
                    .filter(|&band| previous[y + shift][band] == next[y][band])
                    .count()
            })
            .sum();
        matched * 100 >= (body - shift) * BANDS * 83
    });
    let best = verified.next()?;
    // A regular grid can produce multiple convincing shifts. Keep the last
    // accepted frame until there is enough distinct content to disambiguate.
    if verified.any(|other| {
        votes[other] * 3 >= votes[best]
            && u64::from(votes[other]) * (body - best) as u64 * 10
                >= u64::from(votes[best]) * (body - other) as u64 * 9
    }) {
        return None;
    }
    Some((best, footer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    const WIDTH: u32 = 144;
    const BODY: u32 = 140;
    const HEADER: u32 = 18;
    const FOOTER: u32 = 12;

    fn page_row(y: u32, x: u32) -> Rgba<u8> {
        let seed = (y ^ (x / 8)).wrapping_mul(0x9e3779b1).rotate_left(13);
        let [r, g, b, _] = seed.to_le_bytes();
        Rgba([r, g, b, 255])
    }

    fn viewport(offset: u32) -> RgbaImage {
        RgbaImage::from_fn(WIDTH, HEADER + BODY + FOOTER, |x, y| {
            if y < HEADER {
                Rgba([20, 28, 45, 255])
            } else if y >= HEADER + BODY {
                Rgba([50, 30, 22, 255])
            } else {
                page_row(y - HEADER + offset, x)
            }
        })
    }

    #[test]
    fn stitches_physical_rows_and_keeps_one_header_and_footer() {
        let mut stitch = Stitcher::new(viewport(0), 4096);
        assert_eq!(stitch.append(viewport(43)), Update::Appended);
        assert_eq!(stitch.append(viewport(94)), Update::Appended);
        let result = stitch.into_image();
        assert_eq!(result.dimensions(), (WIDTH, HEADER + BODY + FOOTER + 94));
        for (x, y, pixel) in result.enumerate_pixels() {
            if (HEADER..HEADER + BODY + 94).contains(&y) {
                assert_eq!(*pixel, page_row(y - HEADER, x));
            } else if y < HEADER {
                assert_eq!(*pixel, Rgba([20, 28, 45, 255]));
            } else {
                assert_eq!(*pixel, Rgba([50, 30, 22, 255]));
            }
        }
    }

    #[test]
    fn skips_duplicates_and_rejects_gaps_until_the_viewport_overlaps_again() {
        let mut stitch = Stitcher::new(viewport(0), 300);
        assert_eq!(stitch.append(viewport(0)), Update::Unchanged);
        assert_eq!(stitch.append(viewport(122)), Update::NoOverlap);
        assert_eq!(stitch.append(viewport(200)), Update::NoOverlap);
        assert_eq!(stitch.height(), 170);
        assert_eq!(stitch.append(viewport(94)), Update::Appended);
        assert_eq!(stitch.append(viewport(25)), Update::NoOverlap);
        assert_eq!(stitch.append(viewport(140)), Update::Limit);
        assert_eq!(stitch.height(), 264);
    }

    #[test]
    fn refuses_ambiguous_repeating_content() {
        let stripes =
            |offset| RgbaImage::from_fn(WIDTH, 200, |x, y| page_row((y + offset) % 20, x));
        let mut stitch = Stitcher::new(stripes(0), 4096);
        assert_eq!(stitch.append(stripes(7)), Update::NoOverlap);
        assert_eq!(stitch.height(), 200);
    }

    #[test]
    fn ignores_local_repaints_before_continuing_to_scroll() {
        let mut stitch = Stitcher::new(viewport(0), 4096);
        let mut hovering = viewport(0);
        for y in 30..45 {
            for x in 25..30 {
                hovering.put_pixel(x, y, Rgba([45, 130, 224, 255]));
            }
        }
        assert_eq!(stitch.append(hovering), Update::Unchanged);
        assert_eq!(stitch.append(viewport(43)), Update::Appended);
        assert_eq!(stitch.height(), HEADER + BODY + FOOTER + 43);
    }
}
