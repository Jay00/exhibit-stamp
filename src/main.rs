use ab_glyph::{FontRef, PxScale};
use clap::{Parser, ValueEnum};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use image::{Rgba, RgbaImage};
use imageproc::drawing::{
    draw_filled_circle_mut, draw_filled_rect_mut, draw_hollow_rect_mut, draw_line_segment_mut,
    draw_text_mut, text_size,
};

use imageproc::rect::Rect;
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use std::io::Write;
use std::path::PathBuf;

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum ExhibitType {
    Defense,
    Government,
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Stamps an evidence exhibit sticker onto a PDF page."
)]
struct Args {
    #[arg(short, long)]
    input: PathBuf,

    #[arg(short, long)]
    output: PathBuf,

    #[arg(short, long, default_value_t = 1)]
    page: u32,

    /// Exhibit type determines sticker background color
    #[arg(value_enum, short = 't', long, default_value_t = ExhibitType::Defense)]
    exhibit_type: ExhibitType,

    #[arg(short, long, default_value = "EXHIBIT 14")]
    exhibit: String,

    #[arg(long, default_value = "DC SUPERIOR COURT")]
    title: String,

    #[arg(long, default_value = "2026 LLL 000000")]
    case_no: String,

    /// Position X in PDF points from bottom-left
    #[arg(short, long, default_value_t = 350.0)]
    x: f64,

    /// Position Y in PDF points from bottom-left
    #[arg(short, long, default_value_t = 50.0)]
    y: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // 1. Generate sticker with color palette matching exhibit_type
    let img = generate_exhibit_sticker(
        &args.title,
        &args.exhibit,
        &args.case_no,
        &args.exhibit_type,
    )?;

    // 2. Load PDF & embed sticker
    let mut doc = Document::load(&args.input)?;
    let image_obj_id = embed_rgba_image(&mut doc, &img)?;

    // 3. Apply to target page
    stamp_page(
        &mut doc,
        args.page,
        image_obj_id,
        args.x,
        args.y,
        220.0,
        130.0,
    )?;

    doc.save(&args.output)?;
    println!(
        "Applied {:?} exhibit sticker to page {} and saved to {:?}",
        args.exhibit_type, args.page, args.output
    );

    Ok(())
}

fn load_system_font() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use font_kit::family_name::FamilyName;
    use font_kit::properties::Properties;
    use font_kit::source::SystemSource;

    let property = Properties::new();
    let handle = SystemSource::new().select_best_match(&[FamilyName::SansSerif], &property)?;

    let font_data = match handle {
        font_kit::handle::Handle::Path { path, .. } => std::fs::read(path)?,
        font_kit::handle::Handle::Memory { bytes, .. } => bytes.to_vec(),
    };

    Ok(font_data)
}

fn generate_exhibit_sticker(
    title: &str,
    exhibit: &str,
    case_no: &str,
    exhibit_type: &ExhibitType,
) -> Result<RgbaImage, Box<dyn std::error::Error>> {
    let width = 300;
    let height = 200;
    let mut img = RgbaImage::new(width, height);

    // Inside generate_exhibit_sticker:
    let font_bytes = load_system_font()?;
    let font = FontRef::try_from_slice(&font_bytes)?;

    // Black border & text colors
    let border_color = Rgba([0, 0, 0, 255]);
    let text_color = Rgba([0, 0, 0, 255]);
    let transparent = Rgba([0, 0, 0, 0]);

    // Select background color based on party type
    let bg_color = match exhibit_type {
        ExhibitType::Defense => Rgba([173, 216, 230, 255]), // Opaque Light Blue
        ExhibitType::Government => Rgba([255, 235, 120, 255]), // Opaque Light Yellow
    };

    // Select Exhibit Type Text (DEFENSE / GOVERNMENT)
    let exhibit_type_text = match exhibit_type {
        ExhibitType::Defense => "DEFENSE EXHIBIT",
        ExhibitType::Government => "GOVERNMENT EXHIBIT",
    };

    // Initialize canvas as transparent (for space outside the rounded border)
    for p in img.pixels_mut() {
        *p = transparent;
    }

    // Outer and Inner padding for bezel border
    let pad = 8;
    let border_thickness = 5;

    // 1. Fill solid opaque background within rounded rectangle
    draw_rounded_rect_with_border(
        &mut img,
        pad as i32,
        pad as i32,
        width - pad * 2,
        height - pad * 2,
        16,
        border_thickness,
        bg_color,
        border_color,
    );

    // 2. Draw black section dividers
    let line_y1 = 30.0;
    let line_y2 = height as f32 - 30.0 - border_thickness as f32;
    for offset in 0..border_thickness {
        let y1 = line_y1 + offset as f32;
        let y2 = line_y2 + offset as f32;
        draw_line_segment_mut(
            &mut img,
            (pad as f32, y1),
            ((width - pad) as f32, y1),
            border_color,
        );
        draw_line_segment_mut(
            &mut img,
            (pad as f32, y2),
            ((width - pad) as f32, y2),
            border_color,
        );
    }

    // 3. Header Text
    let scale_sm = PxScale::from(20.0);
    let (tw_title, _) = text_size(scale_sm, &font, title);
    draw_text_mut(
        &mut img,
        text_color,
        ((width as i32) - tw_title as i32) / 2,
        5 + scale_sm.y as i32,
        scale_sm,
        &font,
        title,
    );

    // 3.5 DEFENSE / GOVERNMENT Exhibit
    let scale_lg = PxScale::from(20.0);
    let (tw_ex, _) = text_size(scale_lg, &font, exhibit_type_text);
    draw_text_mut(
        &mut img,
        text_color,
        ((width as i32) - tw_ex as i32) / 2,
        70,
        scale_lg,
        &font,
        exhibit_type_text,
    );

    // 4. Main Exhibit Text (Bold Center)
    let scale_lg = PxScale::from(45.0);
    let (tw_ex, _) = text_size(scale_lg, &font, exhibit);
    draw_text_mut(
        &mut img,
        text_color,
        ((width as i32) - tw_ex as i32) / 2,
        100,
        scale_lg,
        &font,
        exhibit,
    );

    // 5. Footer Case Info
    let case_text = format!("CASE NO: {}", case_no);
    let (tw_case, _) = text_size(scale_sm, &font, &case_text);
    draw_text_mut(
        &mut img,
        text_color,
        ((width as i32) - tw_case as i32) / 2,
        160,
        scale_sm,
        &font,
        &case_text,
    );

    Ok(img)
}

pub fn draw_rounded_rect_filled(
    img: &mut RgbaImage,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    radius: u32,
    color: Rgba<u8>,
) {
    let r = radius as i32;
    let w = width as i32;
    let h = height as i32;

    // 1. Central vertical rectangle (spans full height minus corner radii)
    draw_filled_rect_mut(
        img,
        Rect::at(x, y + r).of_size(width, (height - 2 * radius) as u32),
        color,
    );

    // 2. Central horizontal rectangle (spans full width minus corner radii)
    draw_filled_rect_mut(
        img,
        Rect::at(x + r, y).of_size((width - 2 * radius) as u32, height),
        color,
    );

    // 3. Four corner circles
    draw_filled_circle_mut(img, (x + r, y + r), r, color); // Top-Left
    draw_filled_circle_mut(img, (x + w - r - 1, y + r), r, color); // Top-Right
    draw_filled_circle_mut(img, (x + r, y + h - r - 1), r, color); // Bottom-Left
    draw_filled_circle_mut(img, (x + w - r - 1, y + h - r - 1), r, color); // Bottom-Right
}

pub fn draw_rounded_rect_with_border(
    img: &mut RgbaImage,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    radius: u32,
    border_thickness: u32,
    fill_color: Rgba<u8>,
    border_color: Rgba<u8>,
) {
    // Outer shape (Border)
    draw_rounded_rect_filled(img, x, y, width, height, radius, border_color);

    // Inner shape (Fill cutout)
    let b = border_thickness as i32;
    let inner_width = width.saturating_sub(border_thickness * 2);
    let inner_height = height.saturating_sub(border_thickness * 2);
    let inner_radius = radius.saturating_sub(border_thickness);

    if inner_width > 0 && inner_height > 0 {
        draw_rounded_rect_filled(
            img,
            x + b,
            y + b,
            inner_width,
            inner_height,
            inner_radius,
            fill_color,
        );
    }
}

fn embed_rgba_image(
    doc: &mut Document,
    img: &RgbaImage,
) -> Result<lopdf::ObjectId, Box<dyn std::error::Error>> {
    let (width, height) = img.dimensions();
    let mut rgb_bytes = Vec::with_capacity((width * height * 3) as usize);
    let mut alpha_bytes = Vec::with_capacity((width * height) as usize);

    for pixel in img.pixels() {
        rgb_bytes.extend_from_slice(&pixel.0[0..3]);
        alpha_bytes.push(pixel.0[3]);
    }

    let smask_dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => width as i64,
        "Height" => height as i64,
        "ColorSpace" => "DeviceGray",
        "BitsPerComponent" => 8,
        "Filter" => "FlateDecode",
    };
    let smask_id = doc.add_object(Stream::new(smask_dict, compress_bytes(&alpha_bytes)?));

    let image_dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => width as i64,
        "Height" => height as i64,
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
        "Filter" => "FlateDecode",
        "SMask" => smask_id,
    };

    Ok(doc.add_object(Stream::new(image_dict, compress_bytes(&rgb_bytes)?)))
}

fn compress_bytes(bytes: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    encoder.finish()
}

fn stamp_page(
    doc: &mut Document,
    page_num: u32,
    image_obj_id: lopdf::ObjectId,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    let pages = doc.get_pages();
    let page_id = *pages.get(&page_num).ok_or("Page out of bounds")?;
    let img_name = "ExhibitSticker";

    let resources_ref = {
        let page_obj = doc.get_object_mut(page_id)?.as_dict_mut()?;
        if !page_obj.has(b"Resources") {
            page_obj.set("Resources", dictionary! {});
        }
        match page_obj.get(b"Resources")? {
            Object::Reference(id) => Some(*id),
            Object::Dictionary(_) => None,
            _ => return Err("Invalid Resources dictionary".into()),
        }
    };

    let xobjects_ref = if let Some(resources_id) = resources_ref {
        let resources = doc.get_object_mut(resources_id)?.as_dict_mut()?;
        if !resources.has(b"XObject") {
            resources.set("XObject", dictionary! {});
        }
        match resources.get_mut(b"XObject")? {
            Object::Reference(id) => Some(*id),
            Object::Dictionary(xobjects) => {
                xobjects.set(img_name, image_obj_id);
                None
            }
            _ => return Err("Invalid XObject dictionary".into()),
        }
    } else {
        let page_obj = doc.get_object_mut(page_id)?.as_dict_mut()?;
        let resources = page_obj.get_mut(b"Resources")?.as_dict_mut()?;
        if !resources.has(b"XObject") {
            resources.set("XObject", dictionary! {});
        }
        match resources.get_mut(b"XObject")? {
            Object::Reference(id) => Some(*id),
            Object::Dictionary(xobjects) => {
                xobjects.set(img_name, image_obj_id);
                None
            }
            _ => return Err("Invalid XObject dictionary".into()),
        }
    };

    if let Some(xobjects_id) = xobjects_ref {
        doc.get_object_mut(xobjects_id)?
            .as_dict_mut()?
            .set(img_name, image_obj_id);
    }

    let stamp_ops = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    w.into(),
                    0.0.into(),
                    0.0.into(),
                    h.into(),
                    x.into(),
                    y.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(img_name.as_bytes().to_vec())]),
            Operation::new("Q", vec![]),
        ],
    };

    let stamp_stream_id = doc.add_object(Stream::new(dictionary! {}, stamp_ops.encode()?));
    let page_dict = doc.get_object_mut(page_id)?.as_dict_mut()?;

    match page_dict.get_mut(b"Contents") {
        Ok(Object::Array(contents)) => contents.push(Object::Reference(stamp_stream_id)),
        Ok(Object::Reference(id)) => {
            let existing_ref = Object::Reference(*id);
            page_dict.set(
                "Contents",
                vec![existing_ref, Object::Reference(stamp_stream_id)],
            );
        }
        _ => {
            page_dict.set("Contents", vec![Object::Reference(stamp_stream_id)]);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_defense_sticker() {
        let sticker = generate_exhibit_sticker(
            "DC SUPERIOR COURT",
            "15",
            "2025 CF1 123456",
            &ExhibitType::Defense,
        )
        .unwrap();

        sticker.save("target/test_defense_sticker.png").unwrap();
    }

    #[test]
    fn test_generate_government_sticker() {
        let sticker = generate_exhibit_sticker(
            "UNITED STATES DISTRICT COURT",
            "GOVERNMENT EXHIBIT 1",
            "CR-2026-00104",
            &ExhibitType::Government,
        )
        .unwrap();

        sticker.save("target/test_government_sticker.png").unwrap();
    }
}
