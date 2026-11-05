//! Finding a small picture inside a bigger one.
//!
//! Everything here works on 8-bit grayscale and is pure, so the matcher can be
//! tested against drawn rectangles with no screen involved.
//!
//! The search is coarse to fine: the screen and the picture are both shrunk,
//! the best place at that size is found, and only a small window around it is
//! checked at full size. A full-size search of a 1080p screen against a 100-pixel
//! picture would be about two billion byte comparisons; this is a few million,
//! which is the difference between a block that waits responsively and one that
//! stalls the run.

/// A grayscale image: one byte per pixel, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gray {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Gray {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        if pixels.len() != width as usize * height as usize {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    /// Grayscale from 8-bit RGB or RGBA, with the given bytes per pixel.
    pub fn from_bytes(width: u32, height: u32, data: &[u8], stride: usize) -> Option<Self> {
        if width == 0 || height == 0 || stride < 3 {
            return None;
        }
        let count = width as usize * height as usize;
        if data.len() < count * stride {
            return None;
        }
        let mut pixels = Vec::with_capacity(count);
        for index in 0..count {
            let at = index * stride;
            let (r, g, b) = (
                u32::from(data[at]),
                u32::from(data[at + 1]),
                u32::from(data[at + 2]),
            );
            pixels.push(((r * 299 + g * 587 + b * 114) / 1000) as u8);
        }
        Self::new(width, height, pixels)
    }

    fn at(&self, x: u32, y: u32) -> u8 {
        self.pixels[(y as usize) * (self.width as usize) + x as usize]
    }

    /// Shrink by an integer factor, averaging each block.
    pub fn downscale(&self, factor: u32) -> Gray {
        if factor <= 1 {
            return self.clone();
        }
        let width = (self.width / factor).max(1);
        let height = (self.height / factor).max(1);
        let mut pixels = vec![0u8; width as usize * height as usize];
        for y in 0..height {
            for x in 0..width {
                let mut sum = 0u32;
                let mut count = 0u32;
                for dy in 0..factor {
                    let sy = y * factor + dy;
                    if sy >= self.height {
                        break;
                    }
                    for dx in 0..factor {
                        let sx = x * factor + dx;
                        if sx >= self.width {
                            break;
                        }
                        sum += u32::from(self.at(sx, sy));
                        count += 1;
                    }
                }
                pixels[(y as usize) * width as usize + x as usize] = (sum / count.max(1)) as u8;
            }
        }
        Gray {
            width,
            height,
            pixels,
        }
    }
}

/// Where a picture was found, and how alike it was. 1.0 is identical.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub x: i32,
    pub y: i32,
    pub score: f32,
}

/// Look for `needle` anywhere in `haystack`.
///
/// `min_score` is how alike the two must be, from 0 to 1. Anything below it is
/// reported as not found, so a picture that is merely similar — the same button
/// in a different state, a window with different text in it — is not clicked by
/// mistake.
pub fn find(haystack: &Gray, needle: &Gray, min_score: f32) -> Option<Hit> {
    if needle.width > haystack.width || needle.height > haystack.height {
        return None;
    }
    let factor = coarse_factor(needle);
    let (coarse_hay, coarse_needle) = if factor > 1 {
        (haystack.downscale(factor), needle.downscale(factor))
    } else {
        (haystack.clone(), needle.clone())
    };

    let coarse = best(
        &coarse_hay,
        &coarse_needle,
        0,
        0,
        (coarse_hay.width - coarse_needle.width) as i32,
        (coarse_hay.height - coarse_needle.height) as i32,
    )?;

    if factor == 1 {
        return (coarse.score >= min_score).then_some(Hit {
            x: coarse.x,
            y: coarse.y,
            score: coarse.score,
        });
    }

    // The coarse answer is only good to a whole block, so the exact place is
    // looked for a couple of blocks either side of it.
    let around_x = coarse.x * factor as i32;
    let around_y = coarse.y * factor as i32;
    let pad = factor as i32 * 2;
    let x0 = (around_x - pad).max(0);
    let y0 = (around_y - pad).max(0);
    let x1 = (around_x + pad).min((haystack.width - needle.width) as i32);
    let y1 = (around_y + pad).min((haystack.height - needle.height) as i32);
    let fine = best(haystack, needle, x0, y0, x1, y1)?;

    (fine.score >= min_score).then_some(Hit {
        x: fine.x,
        y: fine.y,
        score: fine.score,
    })
}

/// How much the picture may be shrunk before the coarse pass.
///
/// A picture smaller than a couple of blocks would turn into a handful of
/// pixels and match everything, so the factor stops before that.
fn coarse_factor(needle: &Gray) -> u32 {
    for factor in [8, 4, 2] {
        if needle.width / factor >= 6 && needle.height / factor >= 6 {
            return factor;
        }
    }
    1
}

/// The best place in a rectangle of possible top-left corners.
fn best(hay: &Gray, needle: &Gray, x0: i32, y0: i32, x1: i32, y1: i32) -> Option<Hit> {
    if x1 < x0 || y1 < y0 {
        return None;
    }
    let mut top: Option<Hit> = None;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let score = score_at(hay, needle, x as u32, y as u32);
            if top.map_or(true, |hit| score > hit.score) {
                top = Some(Hit { x, y, score });
                // Nothing can beat an exact match.
                if score >= 0.999 {
                    return top;
                }
            }
        }
    }
    top
}

/// How alike the picture is when its top-left corner sits at `x, y`.
fn score_at(hay: &Gray, needle: &Gray, x: u32, y: u32) -> f32 {
    let mut sum: u64 = 0;
    for ny in 0..needle.height {
        let row = (y + ny) as usize * hay.width as usize + x as usize;
        let nrow = ny as usize * needle.width as usize;
        for nx in 0..needle.width as usize {
            let seen = i32::from(hay.pixels[row + nx]);
            let wanted = i32::from(needle.pixels[nrow + nx]);
            sum += u64::from((seen - wanted).unsigned_abs());
        }
    }
    let total = u64::from(needle.width) * u64::from(needle.height) * 255;
    1.0 - (sum as f32 / total as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(width: u32, height: u32, value: u8) -> Gray {
        Gray::new(width, height, vec![value; (width * height) as usize]).unwrap()
    }

    fn block(image: &mut Gray, x: u32, y: u32, width: u32, height: u32, value: u8) {
        for dy in 0..height {
            for dx in 0..width {
                let at = (y + dy) as usize * image.width as usize + (x + dx) as usize;
                image.pixels[at] = value;
            }
        }
    }

    fn crop(image: &Gray, x: u32, y: u32, width: u32, height: u32) -> Gray {
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for dy in 0..height {
            for dx in 0..width {
                pixels.push(image.at(x + dx, y + dy));
            }
        }
        Gray::new(width, height, pixels).unwrap()
    }

    /// A checkerboard, so a match cannot be a run of one flat colour.
    fn checker(width: u32, height: u32, cell: u32) -> Gray {
        let mut image = canvas(width, height, 30);
        for y in 0..height {
            for x in 0..width {
                if ((x / cell) + (y / cell)) % 2 == 0 {
                    image.pixels[(y * width + x) as usize] = 220;
                }
            }
        }
        image
    }

    #[test]
    fn finds_a_picture_where_it_was_drawn() {
        let mut screen = canvas(400, 300, 200);
        let piece = checker(48, 32, 8);
        for dy in 0..piece.height {
            for dx in 0..piece.width {
                let at = (120 + dy) as usize * 400 + (53 + dx) as usize;
                screen.pixels[at] = piece.at(dx, dy);
            }
        }
        let found = find(&screen, &piece, 0.9).expect("the picture is there");
        assert_eq!((found.x, found.y), (53, 120));
        assert!(found.score > 0.99, "score was {}", found.score);
    }

    /// A picture that is not on screen must not be found somewhere close.
    #[test]
    fn does_not_invent_a_picture_that_is_not_there() {
        let screen = canvas(400, 300, 200);
        let piece = checker(48, 32, 8);
        assert_eq!(find(&screen, &piece, 0.9), None);
    }

    #[test]
    fn a_picture_bigger_than_the_screen_is_never_found() {
        let screen = canvas(40, 30, 200);
        let piece = checker(80, 60, 8);
        assert_eq!(find(&screen, &piece, 0.5), None);
    }

    /// The coarse pass has to land close enough that the fine pass, which only
    /// looks a couple of blocks either side, still sees the real spot.
    #[test]
    fn a_picture_found_at_a_coarse_scale_is_refined_to_the_pixel() {
        let mut screen = canvas(640, 480, 90);
        let piece = checker(64, 64, 4);
        for dy in 0..piece.height {
            for dx in 0..piece.width {
                let at = (301 + dy) as usize * 640 + (205 + dx) as usize;
                screen.pixels[at] = piece.at(dx, dy);
            }
        }
        let found = find(&screen, &piece, 0.9).expect("the picture is there");
        assert_eq!((found.x, found.y), (205, 301));
    }

    #[test]
    fn a_picture_cropped_from_a_screen_is_identical_to_the_pixels_it_came_from() {
        let screen = checker(200, 160, 5);
        let piece = crop(&screen, 40, 30, 20, 20);
        let found = find(&screen, &piece, 0.999).expect("an exact copy is found");
        assert!((found.score - 1.0).abs() < 0.001);
    }

    #[test]
    fn shrinking_a_flat_image_keeps_it_flat() {
        let image = canvas(64, 64, 77);
        let small = image.downscale(8);
        assert_eq!((small.width, small.height), (8, 8));
        assert!(small.pixels.iter().all(|value| *value == 77));
    }
}
