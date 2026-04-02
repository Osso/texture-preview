use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use image::{DynamicImage, ImageBuffer, ImageOutputFormat, RgbaImage};

#[derive(Parser)]
#[command(about = "Convert game texture formats to JPEG for preview")]
struct Args {
    /// Input texture file (BLP, KTX2)
    input: PathBuf,
    /// Output JPEG file
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let ext = args
        .input
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    let image = match ext.as_str() {
        "blp" => decode_blp(&args.input)?,
        "ktx2" => decode_ktx2(&args.input)?,
        other => bail!("unsupported format: {other}"),
    };

    write_preview(&image, &args.output)?;

    Ok(())
}

fn write_preview(image: &DynamicImage, path: &PathBuf) -> Result<()> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    image
        .write_to(&mut writer, ImageOutputFormat::Jpeg(90))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn decode_blp(path: &PathBuf) -> Result<DynamicImage> {
    let data = std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let blp = image_blp::parser::load_blp_from_buf(&data)
        .map_err(|e| anyhow::anyhow!("BLP parse error: {e}"))?;
    let image = image_blp::convert::blp_to_image(&blp, 0)
        .map_err(|e| anyhow::anyhow!("BLP decode error: {e}"))?;
    Ok(image)
}

fn decode_ktx2(path: &PathBuf) -> Result<DynamicImage> {
    let data = std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let reader = ktx2::Reader::new(&data).map_err(|e| anyhow::anyhow!("KTX2 parse error: {e}"))?;
    let header = reader.header();

    let width = header.pixel_width;
    let height = header.pixel_height;

    let levels: Vec<_> = reader.levels().collect();
    let level_data = levels.first().context("KTX2 has no mip levels")?;

    let format = header
        .format
        .context("KTX2 has no format specified (Basis Universal not supported)")?;
    let rgba = decode_ktx2_pixels(format, level_data.data, width, height)?;

    let buf: RgbaImage = ImageBuffer::from_raw(width, height, rgba)
        .context("failed to create image buffer from KTX2 data")?;

    Ok(DynamicImage::ImageRgba8(buf))
}

fn decode_uncompressed_rgba(data: &[u8], pixel_count: usize) -> Vec<u8> {
    data[..pixel_count * 4].to_vec()
}

fn decode_uncompressed_rgb(data: &[u8], pixel_count: usize) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for chunk in data[..pixel_count * 3].chunks_exact(3) {
        rgba.extend_from_slice(chunk);
        rgba.push(255);
    }
    rgba
}

fn decode_block_compressed(
    format: ktx2::Format,
    data: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<u32>> {
    let mut buf = vec![0u32; width * height];
    let e = |s: &str| anyhow::anyhow!("{s}");
    use ktx2::Format;
    match format {
        Format::BC1_RGB_UNORM_BLOCK
        | Format::BC1_RGB_SRGB_BLOCK
        | Format::BC1_RGBA_UNORM_BLOCK
        | Format::BC1_RGBA_SRGB_BLOCK => {
            texture2ddecoder::decode_bc1(data, width, height, &mut buf).map_err(e)?;
        }
        Format::BC3_UNORM_BLOCK | Format::BC3_SRGB_BLOCK => {
            texture2ddecoder::decode_bc3(data, width, height, &mut buf).map_err(e)?;
        }
        Format::BC4_UNORM_BLOCK | Format::BC4_SNORM_BLOCK => {
            texture2ddecoder::decode_bc4(data, width, height, &mut buf).map_err(e)?;
        }
        Format::BC5_UNORM_BLOCK | Format::BC5_SNORM_BLOCK => {
            texture2ddecoder::decode_bc5(data, width, height, &mut buf).map_err(e)?;
        }
        Format::BC7_UNORM_BLOCK | Format::BC7_SRGB_BLOCK => {
            texture2ddecoder::decode_bc7(data, width, height, &mut buf).map_err(e)?;
        }
        Format::ETC2_R8G8B8_UNORM_BLOCK | Format::ETC2_R8G8B8_SRGB_BLOCK => {
            texture2ddecoder::decode_etc1(data, width, height, &mut buf).map_err(e)?;
        }
        Format::ETC2_R8G8B8A8_UNORM_BLOCK | Format::ETC2_R8G8B8A8_SRGB_BLOCK => {
            texture2ddecoder::decode_etc2_rgba8(data, width, height, &mut buf).map_err(e)?;
        }
        Format::ASTC_4x4_UNORM_BLOCK | Format::ASTC_4x4_SRGB_BLOCK => {
            texture2ddecoder::decode_astc(data, width, height, 4, 4, &mut buf).map_err(e)?;
        }
        other => bail!("unsupported KTX2 pixel format: {other:?}"),
    }
    Ok(buf)
}

fn bgra_u32_to_rgba_u8(buf: &[u32]) -> Vec<u8> {
    buf.iter()
        .flat_map(|&pixel| {
            let b = (pixel & 0xFF) as u8;
            let g = ((pixel >> 8) & 0xFF) as u8;
            let r = ((pixel >> 16) & 0xFF) as u8;
            let a = ((pixel >> 24) & 0xFF) as u8;
            [r, g, b, a]
        })
        .collect()
}

fn decode_ktx2_pixels(
    format: ktx2::Format,
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let pixel_count = (width * height) as usize;

    use ktx2::Format;
    match format {
        Format::R8G8B8A8_UNORM | Format::R8G8B8A8_SRGB => {
            Ok(decode_uncompressed_rgba(data, pixel_count))
        }
        Format::R8G8B8_UNORM | Format::R8G8B8_SRGB => {
            Ok(decode_uncompressed_rgb(data, pixel_count))
        }
        _ => {
            let buf =
                decode_block_compressed(format, data, width as usize, height as usize)?;
            Ok(bgra_u32_to_rgba_u8(&buf))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::write_preview;
    use image::DynamicImage;

    #[test]
    fn writes_jpeg_without_output_extension() {
        let path = std::env::temp_dir().join(format!(
            "texture-preview-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        write_preview(&DynamicImage::new_rgba8(1, 1), &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(&[0xFF, 0xD8]));

        let _ = std::fs::remove_file(path);
    }
}
