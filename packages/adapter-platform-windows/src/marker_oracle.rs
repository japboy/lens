//! Independent pixel oracle for the fixed 96-DPI, 64×40 validation bitmap.
//! This module knows neither UIA nor the adapter's coordinate conversion.

pub const WIDTH: u32 = 64;
pub const HEIGHT: u32 = 40;

#[cfg(all(windows, target_arch = "x86_64"))]
pub fn verify_pngs(
    frame: &[u8],
    capture: &[u8],
    reported: Rect,
    phase: Phase,
) -> Result<Rect, String> {
    let decode = |data| {
        image::load_from_memory_with_format(data, image::ImageFormat::Png)
            .map(image::DynamicImage::into_rgba8)
            .map_err(|error| error.to_string())
    };
    let frame = decode(frame)?;
    let capture = decode(capture)?;
    verify(
        Rgba::new(frame.width(), frame.height(), frame.as_raw())?,
        Rgba::new(capture.width(), capture.height(), capture.as_raw())?,
        reported,
        phase,
    )
    .map_err(str::to_owned)
}

#[derive(Clone, Copy, Debug)]
pub enum Phase {
    A,
    B,
    Updated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub struct Rgba<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

impl<'a> Rgba<'a> {
    pub fn new(width: u32, height: u32, pixels: &'a [u8]) -> Result<Self, &'static str> {
        let length = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| usize::try_from(n).ok());
        if width == 0 || height == 0 || length != Some(pixels.len()) {
            return Err("invalid RGBA dimensions or byte length");
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    fn pixel(&self, x: u32, y: u32) -> &[u8] {
        let offset = ((u64::from(y) * u64::from(self.width) + u64::from(x)) * 4) as usize;
        &self.pixels[offset..offset + 4]
    }
}

pub fn expected_pixel(phase: Phase, x: u32, y: u32) -> [u8; 4] {
    let rgb = if x == 0 || x == WIDTH - 1 || y == 0 || y == HEIGHT - 1 {
        [245, 245, 245]
    } else if x < 9 && y < 7 {
        [231, 31, 39]
    } else if x >= 53 && y < 5 {
        [17, 211, 224]
    } else if x < 6 && y >= 29 {
        [247, 213, 19]
    } else if x >= 47 && y >= 31 {
        [145, 37, 211]
    } else if (28..31).contains(&x) && (9..28).contains(&y) {
        [9, 12, 15]
    } else {
        match phase {
            Phase::A => [30, 80, 220],
            Phase::B => [40, 210, 70],
            Phase::Updated => [170, 115, 25],
        }
    };
    [rgb[0], rgb[1], rgb[2], 255]
}

fn matches(frame: &Rgba<'_>, x: u32, y: u32, phase: Phase) -> bool {
    (0..HEIGHT)
        .all(|dy| (0..WIDTH).all(|dx| frame.pixel(x + dx, y + dy) == expected_pixel(phase, dx, dy)))
}

/// Locate the unique complete marker from pixels alone, then compare all four
/// reported crop edges and every cropped pixel. No fitting or tolerance tuning.
pub fn verify(
    raw: Rgba<'_>,
    crop: Rgba<'_>,
    reported: Rect,
    phase: Phase,
) -> Result<Rect, &'static str> {
    if raw.width < WIDTH || raw.height < HEIGHT || crop.width != WIDTH || crop.height != HEIGHT {
        return Err("fixture marker dimensions must be exactly 64 by 40");
    }
    let mut found = None;
    for y in 0..=raw.height - HEIGHT {
        for x in 0..=raw.width - WIDTH {
            if raw.pixel(x, y) == [245, 245, 245, 255] && matches(&raw, x, y, phase) {
                let rect = Rect {
                    x,
                    y,
                    width: WIDTH,
                    height: HEIGHT,
                };
                if found.replace(rect).is_some() {
                    return Err("raw frame contains more than one matching fixture marker");
                }
            }
        }
    }
    let actual = found.ok_or("raw frame has no complete expected fixture marker")?;
    if reported != actual {
        return Err("reported crop does not match independently located marker edges");
    }
    if !matches(&crop, 0, 0, phase) {
        return Err("cropped PNG does not contain the complete asymmetric marker");
    }
    Ok(actual)
}
