//! Clipboard images: raw RGBA pixels, stored as PNG.

use std::io::Cursor;

/// The largest image handled, in pixels: 64 megapixels, 256 MB as RGBA.
pub const MAX_IMAGE_PIXELS: u64 = 64_000_000;

/// Image failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    /// The data is not a usable image.
    #[error("invalid image: {0}")]
    Invalid(String),
    /// The image is larger than [`MAX_IMAGE_PIXELS`].
    #[error(
        "the image is {width} x {height} pixels, larger than the 64 megapixels passalong handles"
    )]
    TooLarge {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
}

/// An image as 8-bit RGBA pixels, row by row: what clipboards exchange.
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RgbaImage({} x {})", self.width, self.height)
    }
}

impl RgbaImage {
    /// An image of `width` x `height` pixels.
    ///
    /// # Errors
    ///
    /// [`ImageError::TooLarge`] above [`MAX_IMAGE_PIXELS`];
    /// [`ImageError::Invalid`] for an empty image or a buffer that is not
    /// exactly `width * height * 4` bytes.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, ImageError> {
        check_size(width, height)?;
        let expected = u64::from(width) * u64::from(height) * 4;
        if rgba.len() as u64 != expected {
            return Err(ImageError::Invalid(format!(
                "{width} x {height} pixels need {expected} bytes, got {}",
                rgba.len()
            )));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels, four bytes each.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The pixel buffer.
    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }
}

fn check_size(width: u32, height: u32) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Invalid("the image has no pixels".to_owned()));
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(ImageError::TooLarge { width, height });
    }
    Ok(())
}

/// Encodes an image as an 8-bit RGBA PNG.
///
/// # Errors
///
/// [`ImageError::Invalid`] when the encoder fails.
pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, ImageError> {
    let invalid = |err: png::EncodingError| ImageError::Invalid(err.to_string());
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(invalid)?;
    writer.write_image_data(&image.rgba).map_err(invalid)?;
    writer.finish().map_err(invalid)?;
    Ok(out)
}

/// Decodes a PNG of any color type and bit depth into 8-bit RGBA. The size
/// is checked against [`MAX_IMAGE_PIXELS`] before any pixels are read.
///
/// # Errors
///
/// [`ImageError::TooLarge`] or [`ImageError::Invalid`].
pub fn decode_png(png_bytes: &[u8]) -> Result<RgbaImage, ImageError> {
    let invalid = |err: png::DecodingError| ImageError::Invalid(err.to_string());
    let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    decoder.set_limits(png::Limits {
        bytes: usize::try_from(MAX_IMAGE_PIXELS * 4 + (1 << 20)).unwrap_or(usize::MAX),
    });
    let mut reader = decoder.read_info().map_err(invalid)?;
    let (width, height) = (reader.info().width, reader.info().height);
    check_size(width, height)?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| ImageError::Invalid("the image is too large for this machine".to_owned()))?;
    let mut buf = vec![0_u8; size];
    let frame = reader.next_frame(&mut buf).map_err(invalid)?;
    buf.truncate(frame.buffer_size());
    let rgba = match frame.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[g, a]| [g, g, g, a])
            .collect(),
        png::ColorType::Indexed => {
            return Err(ImageError::Invalid("unexpanded palette image".to_owned()));
        }
    };
    RgbaImage::new(width, height, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RgbaImage {
        let rgba: Vec<u8> = (0..3 * 2 * 4).map(|i| (i * 37 % 256) as u8).collect();
        RgbaImage::new(3, 2, rgba).unwrap()
    }

    #[test]
    fn images_round_trip_through_png_byte_for_byte() {
        let image = sample();
        let png = encode_png(&image).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(decode_png(&png).unwrap(), image);
    }

    #[test]
    fn the_pixel_buffer_must_match_the_size() {
        assert!(matches!(
            RgbaImage::new(3, 2, vec![0; 23]),
            Err(ImageError::Invalid(_))
        ));
        assert!(matches!(
            RgbaImage::new(0, 2, vec![]),
            Err(ImageError::Invalid(_))
        ));
    }

    fn encode_with(color: png::ColorType, width: u32, height: u32, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(data)
            .unwrap();
        out
    }

    #[test]
    fn grey_and_rgb_images_become_opaque_rgba() {
        let grey = decode_png(&encode_with(png::ColorType::Grayscale, 2, 1, &[10, 200])).unwrap();
        assert_eq!(grey.rgba(), [10, 10, 10, 255, 200, 200, 200, 255]);
        let rgb = decode_png(&encode_with(png::ColorType::Rgb, 1, 1, &[1, 2, 3])).unwrap();
        assert_eq!(rgb.rgba(), [1, 2, 3, 255]);
        let grey_alpha =
            decode_png(&encode_with(png::ColorType::GrayscaleAlpha, 1, 1, &[7, 9])).unwrap();
        assert_eq!(grey_alpha.rgba(), [7, 7, 7, 9]);
    }

    /// CRC-32 as PNG chunks use it.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFF_u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn images_above_the_pixel_limit_are_refused_before_decoding() {
        let mut png = encode_png(&sample()).unwrap();
        // IHDR data starts after the 8-byte signature, 4-byte length, and
        // 4-byte type: claim 10000 x 10000 pixels and fix up the CRC.
        png[16..20].copy_from_slice(&10_000_u32.to_be_bytes());
        png[20..24].copy_from_slice(&10_000_u32.to_be_bytes());
        let crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        match decode_png(&png) {
            Err(ImageError::TooLarge {
                width: 10_000,
                height: 10_000,
            }) => {}
            other => panic!("unexpected {other:?}"),
        }
        let err = ImageError::TooLarge {
            width: 10_000,
            height: 10_000,
        };
        assert!(err.to_string().contains("64 megapixels"), "{err}");
    }

    #[test]
    fn malformed_data_is_refused() {
        assert!(matches!(
            decode_png(b"not a png"),
            Err(ImageError::Invalid(_))
        ));
    }
}
