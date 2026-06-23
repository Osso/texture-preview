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

#[cfg(not(test))]
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
            let buf = decode_block_compressed(format, data, width as usize, height as usize)?;
            Ok(bgra_u32_to_rgba_u8(&buf))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use image::DynamicImage;

    #[test]
    fn parses_input_and_output_paths() {
        let args = Args::try_parse_from(["texture-preview", "input.ktx2", "out.jpg"]).unwrap();
        assert_eq!(args.input, PathBuf::from("input.ktx2"));
        assert_eq!(args.output, PathBuf::from("out.jpg"));
    }

    #[test]
    fn writes_jpeg_without_output_extension() {
        let path =
            std::env::temp_dir().join(format!("texture-preview-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);

        write_preview(&DynamicImage::new_rgba8(1, 1), &path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(&[0xFF, 0xD8]));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn write_preview_reports_create_errors() {
        let dir = std::env::temp_dir().join(format!("texture-preview-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let err = write_preview(&DynamicImage::new_rgba8(1, 1), &dir).unwrap_err();

        assert!(err.to_string().contains("failed to create"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn uncompressed_rgba_copies_exact_pixel_bytes() {
        let data = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        assert_eq!(
            decode_uncompressed_rgba(&data, 2),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn uncompressed_rgb_adds_opaque_alpha() {
        let data = [10, 20, 30, 40, 50, 60, 70];
        assert_eq!(
            decode_uncompressed_rgb(&data, 2),
            vec![10, 20, 30, 255, 40, 50, 60, 255]
        );
    }

    #[test]
    fn converts_bgra_words_to_rgba_bytes() {
        const FIRST_BGRA_PIXEL: u32 = 0x44332211;
        const SECOND_BGRA_PIXEL: u32 = 0xDDCCBBAA;
        let pixels = [FIRST_BGRA_PIXEL, SECOND_BGRA_PIXEL];
        assert_eq!(
            bgra_u32_to_rgba_u8(&pixels),
            vec![0x33, 0x22, 0x11, 0x44, 0xCC, 0xBB, 0xAA, 0xDD]
        );
    }

    #[test]
    fn decodes_uncompressed_ktx2_pixel_formats() {
        use ktx2::Format;

        let rgba = decode_ktx2_pixels(Format::R8G8B8A8_UNORM, &[1, 2, 3, 4], 1, 1).unwrap();
        assert_eq!(rgba, vec![1, 2, 3, 4]);

        let rgb = decode_ktx2_pixels(Format::R8G8B8_SRGB, &[5, 6, 7], 1, 1).unwrap();
        assert_eq!(rgb, vec![5, 6, 7, 255]);
    }

    #[test]
    fn rejects_unsupported_block_format() {
        let err = decode_block_compressed(ktx2::Format::R4G4_UNORM_PACK8, &[], 1, 1).unwrap_err();
        assert!(err.to_string().contains("unsupported KTX2 pixel format"));
    }

    #[test]
    fn supported_block_formats_report_decoder_errors_for_invalid_payloads() {
        use ktx2::Format;

        let formats = [
            Format::BC1_RGB_UNORM_BLOCK,
            Format::BC1_RGB_SRGB_BLOCK,
            Format::BC1_RGBA_UNORM_BLOCK,
            Format::BC1_RGBA_SRGB_BLOCK,
            Format::BC3_UNORM_BLOCK,
            Format::BC3_SRGB_BLOCK,
            Format::BC4_UNORM_BLOCK,
            Format::BC4_SNORM_BLOCK,
            Format::BC5_UNORM_BLOCK,
            Format::BC5_SNORM_BLOCK,
            Format::BC7_UNORM_BLOCK,
            Format::BC7_SRGB_BLOCK,
            Format::ETC2_R8G8B8_UNORM_BLOCK,
            Format::ETC2_R8G8B8_SRGB_BLOCK,
            Format::ETC2_R8G8B8A8_UNORM_BLOCK,
            Format::ETC2_R8G8B8A8_SRGB_BLOCK,
            Format::ASTC_4x4_UNORM_BLOCK,
            Format::ASTC_4x4_SRGB_BLOCK,
        ];

        for format in formats {
            let err = decode_block_compressed(format, &[], 4, 4).unwrap_err();
            assert!(!err.to_string().is_empty());
        }
    }

    #[test]
    fn decodes_valid_bc1_block_to_rgba_bytes() {
        let block = [0_u8; 8];

        let bgra =
            decode_block_compressed(ktx2::Format::BC1_RGB_UNORM_BLOCK, &block, 4, 4).unwrap();
        let rgba = decode_ktx2_pixels(ktx2::Format::BC1_RGB_UNORM_BLOCK, &block, 4, 4).unwrap();

        assert_eq!(bgra.len(), 16);
        assert_eq!(rgba.len(), 64);
    }

    #[test]
    fn decodes_minimal_uncompressed_ktx2_file() {
        let path =
            std::env::temp_dir().join(format!("texture-preview-valid-{}.ktx2", std::process::id()));
        std::fs::write(&path, minimal_rgba_ktx2([1, 2, 3, 4])).unwrap();

        let image = decode_ktx2(&path).unwrap();
        let rgba = image.to_rgba8();

        assert_eq!(rgba.dimensions(), (1, 1));
        assert_eq!(rgba.get_pixel(0, 0).0, [1, 2, 3, 4]);
        std::fs::remove_file(path).unwrap();
    }

    fn minimal_rgba_ktx2(pixel: [u8; 4]) -> Vec<u8> {
        let header_len = ktx2::Header::LENGTH as u64;
        let level_index_len = ktx2::LevelIndex::LENGTH as u64;
        let dfd_len = 4_u64;
        let dfd_offset = header_len + level_index_len;
        let pixel_offset = dfd_offset + dfd_len;
        let header = minimal_ktx2_header(dfd_offset, dfd_len);
        let level = minimal_ktx2_level(pixel_offset, pixel.len() as u64);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header.as_bytes());
        bytes.extend_from_slice(&level.as_bytes());
        bytes.extend_from_slice(&(dfd_len as u32).to_le_bytes());
        bytes.extend_from_slice(&pixel);
        bytes
    }

    fn minimal_ktx2_header(dfd_offset: u64, dfd_len: u64) -> ktx2::Header {
        ktx2::Header {
            format: Some(ktx2::Format::R8G8B8A8_UNORM),
            type_size: 1,
            pixel_width: 1,
            pixel_height: 1,
            pixel_depth: 0,
            layer_count: 0,
            face_count: 1,
            level_count: 1,
            supercompression_scheme: None,
            index: ktx2::Index {
                dfd_byte_offset: dfd_offset as u32,
                dfd_byte_length: dfd_len as u32,
                kvd_byte_offset: 0,
                kvd_byte_length: 0,
                sgd_byte_offset: 0,
                sgd_byte_length: 0,
            },
        }
    }

    fn minimal_ktx2_level(pixel_offset: u64, pixel_len: u64) -> ktx2::LevelIndex {
        ktx2::LevelIndex {
            byte_offset: pixel_offset,
            byte_length: pixel_len,
            uncompressed_byte_length: pixel_len,
        }
    }

    #[test]
    fn decode_file_errors_include_path_context() {
        let path = PathBuf::from("/definitely/missing/texture.ktx2");
        let err = decode_ktx2(&path).unwrap_err();
        assert!(err.to_string().contains("failed to read"));

        let blp_path = PathBuf::from("/definitely/missing/texture.blp");
        let err = decode_blp(&blp_path).unwrap_err();
        assert!(err.to_string().contains("failed to read"));
    }

    #[test]
    fn invalid_texture_payloads_report_parse_errors() {
        let ktx_path = std::env::temp_dir().join(format!(
            "texture-preview-invalid-{}.ktx2",
            std::process::id()
        ));
        let blp_path = std::env::temp_dir().join(format!(
            "texture-preview-invalid-{}.blp",
            std::process::id()
        ));
        std::fs::write(&ktx_path, b"not ktx2").unwrap();
        std::fs::write(&blp_path, b"not blp").unwrap();

        assert!(
            decode_ktx2(&ktx_path)
                .unwrap_err()
                .to_string()
                .contains("KTX2 parse error")
        );
        assert!(
            decode_blp(&blp_path)
                .unwrap_err()
                .to_string()
                .contains("BLP parse error")
        );

        std::fs::remove_file(ktx_path).unwrap();
        std::fs::remove_file(blp_path).unwrap();
    }
}
