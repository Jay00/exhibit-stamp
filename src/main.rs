use clap::{Parser, ValueEnum};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use std::path::{Path, PathBuf};

#[derive(ValueEnum, Clone, Debug, PartialEq)]
pub enum ExhibitType {
    Defense,
    Government,
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Stamps a native vector evidence exhibit sticker onto a PDF."
)]
struct Args {
    #[arg(short, long)]
    input: PathBuf,

    #[arg(short, long)]
    output: Option<PathBuf>,

    #[arg(short, long, default_value_t = 1)]
    page: u32,

    #[arg(value_enum, short = 't', long, default_value_t = ExhibitType::Defense)]
    exhibit_type: ExhibitType,

    #[arg(short, long, default_value = "14")]
    exhibit_num: String,

    #[arg(long)]
    header: Option<String>,

    #[arg(long, default_value = "CR-2026-00892")]
    footer: String,

    /// Distance from the stamp's right edge to the page's right edge, in PDF points (1/72 inch)
    #[arg(short, long, default_value_t = 10.0)]
    right_margin: f64,

    /// Distance from the stamp's bottom edge to the page's bottom edge, in PDF points (1/72 inch)
    #[arg(short, long, default_value_t = 0.0)]
    bottom_margin: f64,

    /// Width in PDF points (1/72 inch)
    #[arg(short, long, default_value_t = 80.0)]
    width: f64,

    /// Height in PDF points (1/72 inch)
    #[arg(short, long, default_value_t = 50.0)]
    height: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut doc = Document::load(&args.input)?;

    stamp_vector_exhibit(
        &mut doc,
        args.page,
        &args.header,
        &args.exhibit_num,
        &args.footer,
        &args.exhibit_type,
        args.right_margin,
        args.bottom_margin,
        args.width,  // width in points
        args.height, // height in points
    )?;

    let output = args
        .output
        .unwrap_or_else(|| stamped_output_path(&args.input));
    doc.save(&output)?;
    println!("Vector exhibit stamp applied cleanly to {:?}", output);
    Ok(())
}

fn stamped_output_path(input: &Path) -> PathBuf {
    let mut file_name = input.file_stem().unwrap_or_default().to_os_string();
    file_name.push("_stamped");
    if let Some(extension) = input.extension() {
        file_name.push(".");
        file_name.push(extension);
    }
    input.with_file_name(file_name)
}

/// Returns the (x_min, y_min, x_max, y_max) MediaBox of a page, following /Parent inheritance.
fn page_media_box(
    doc: &Document,
    page_id: lopdf::ObjectId,
) -> Result<[f64; 4], Box<dyn std::error::Error>> {
    let mut current = page_id;
    loop {
        let dict = doc.get_object(current)?.as_dict()?;
        if let Ok(obj) = dict.get(b"MediaBox") {
            let obj = match obj {
                Object::Reference(id) => doc.get_object(*id)?,
                other => other,
            };
            let nums: Vec<f64> = obj
                .as_array()?
                .iter()
                .map(|o| match o {
                    Object::Integer(i) => Ok(*i as f64),
                    Object::Real(r) => Ok(*r as f64),
                    _ => Err("Invalid MediaBox entry"),
                })
                .collect::<Result<_, _>>()?;
            if nums.len() != 4 {
                return Err("MediaBox must have 4 entries".into());
            }
            return Ok([nums[0], nums[1], nums[2], nums[3]]);
        }
        current = dict.get(b"Parent")?.as_reference()?;
    }
}

/// Returns the page's /Rotate value normalized to 0, 90, 180 or 270, following /Parent inheritance.
fn page_rotation(doc: &Document, page_id: lopdf::ObjectId) -> i64 {
    let mut current = page_id;
    while let Ok(dict) = doc.get_object(current).and_then(|o| o.as_dict()) {
        if let Ok(obj) = dict.get(b"Rotate") {
            let obj = match obj {
                Object::Reference(id) => doc.get_object(*id).unwrap_or(obj),
                other => other,
            };
            if let Object::Integer(r) = obj {
                return r.rem_euclid(360);
            }
            return 0;
        }
        match dict.get(b"Parent").and_then(|p| p.as_reference()) {
            Ok(parent) => current = parent,
            Err(_) => break,
        }
    }
    0
}

/// Helper to navigate/create nested PDF dictionaries without borrow conflicts
fn get_or_create_dict(
    doc: &mut Document,
    parent_id: lopdf::ObjectId,
    key: &[u8],
) -> Result<lopdf::ObjectId, Box<dyn std::error::Error>> {
    let child_obj = {
        let parent = doc.get_object_mut(parent_id)?.as_dict_mut()?;
        if !parent.has(key) {
            parent.set(key, dictionary! {});
        }
        parent.get(key)?.clone()
    };

    match child_obj {
        Object::Reference(id) => Ok(id),
        Object::Dictionary(dict) => {
            let new_id = doc.add_object(dict);
            doc.get_object_mut(parent_id)?
                .as_dict_mut()?
                .set(key, new_id);
            Ok(new_id)
        }
        _ => Err(format!(
            "Expected Dictionary or Reference for key {:?}",
            String::from_utf8_lossy(key)
        )
        .into()),
    }
}

pub fn stamp_vector_exhibit(
    doc: &mut Document,
    page_num: u32,
    header: &Option<String>,
    exhibit_num: &str,
    footer: &str,
    exhibit_type: &ExhibitType,
    right_margin: f64,
    bottom_margin: f64,
    w: f64,
    h: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    let pages = doc.get_pages();
    let page_id = *pages.get(&page_num).ok_or("Page index out of bounds")?;

    let [x_min, y_min, x_max, y_max] = page_media_box(doc, page_id)?;
    let (x_min, x_max) = (x_min.min(x_max), x_min.max(x_max));
    let (y_min, y_max) = (y_min.min(y_max), y_min.max(y_max));

    // Stamp is laid out in the visual (rotated) page space, then mapped to user space with a `cm`.
    let rotation = page_rotation(doc, page_id);
    let (visual_w, _visual_h) = if rotation % 180 == 0 {
        (x_max - x_min, y_max - y_min)
    } else {
        (y_max - y_min, x_max - x_min)
    };
    let matrix: [f64; 6] = match rotation {
        90 => [0.0, 1.0, -1.0, 0.0, x_max, y_min],
        180 => [-1.0, 0.0, 0.0, -1.0, x_max, y_max],
        270 => [0.0, -1.0, 1.0, 0.0, x_min, y_max],
        _ => [1.0, 0.0, 0.0, 1.0, x_min, y_min],
    };
    let x = visual_w - right_margin - w;
    let y = bottom_margin;

    // 1. Register a standard Helvetica-Bold font in Page Resources
    let font_alias = "StampFontBold";
    let font_dict = dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica-Bold",
    };
    let font_id = doc.add_object(font_dict);

    // Safely traverse/create Page -> Resources -> Font without borrow checker issues
    let res_id = get_or_create_dict(doc, page_id, b"Resources")?;
    let font_dict_id = get_or_create_dict(doc, res_id, b"Font")?;
    doc.get_object_mut(font_dict_id)?
        .as_dict_mut()?
        .set(font_alias, font_id);

    // 2. Select Background RGB Color based on exhibit type
    let (bg_r, bg_g, bg_b) = match exhibit_type {
        ExhibitType::Defense => (0.68, 0.85, 0.90), // Light Blue
        ExhibitType::Government => (1.00, 0.92, 0.47), // Light Yellow
    };

    // 3. Build Vector Graphics Operators
    let mut ops = Vec::new();
    ops.push(Operation::new("q", vec![])); // Push graphics state
    ops.push(Operation::new(
        "cm",
        matrix.iter().map(|v| (*v).into()).collect(),
    ));

    // Background Fill
    ops.push(Operation::new(
        "rg",
        vec![bg_r.into(), bg_g.into(), bg_b.into()],
    ));
    append_rounded_rect_path(&mut ops, x, y, w, h, 10.0);
    ops.push(Operation::new("f", vec![]));

    // Outer Border Line
    ops.push(Operation::new(
        "RG",
        vec![0.0.into(), 0.0.into(), 0.0.into()],
    ));
    ops.push(Operation::new("w", vec![3.0.into()])); // Outer border line thickness (3.0pt)
    append_rounded_rect_path(&mut ops, x, y, w, h, 10.0);
    ops.push(Operation::new("S", vec![]));

    // Horizontal Divider Lines
    let line1_y = y + (h * 0.74);
    let line2_y = y + (h * 0.26);

    // -------------------------------------------------------------
    // Set Divider Line Thickness (e.g., 1.5pt for a thinner line)
    // -------------------------------------------------------------
    let divider_thickness = 0.5;
    ops.push(Operation::new("w", vec![divider_thickness.into()]));

    ops.push(Operation::new("m", vec![x.into(), line1_y.into()]));
    ops.push(Operation::new("l", vec![(x + w).into(), line1_y.into()]));
    ops.push(Operation::new("m", vec![x.into(), line2_y.into()]));
    ops.push(Operation::new("l", vec![(x + w).into(), line2_y.into()]));
    ops.push(Operation::new("S", vec![]));

    // -------------------------------------------------------------
    // 4. Calculate Vertically Centered Baselines for Each Zone
    // -------------------------------------------------------------

    // TOP ZONE: header
    let font_size_top = 6.0;
    let top_zone_center = (line1_y + y + h) / 2.0;
    let top_y = top_zone_center - (font_size_top * 0.35);

    // The header text will usually be determined by the exhibit type, but can be overridden by the user
    let header = header.as_deref().unwrap_or(match exhibit_type {
        ExhibitType::Defense => "DEFENSE EXHIBIT",
        ExhibitType::Government => "GOVERNMENT EXHIBIT",
    });

    let top_x = center_text_offset(header, font_size_top, x, w);

    // MIDDLE ZONE: Exhibit Number (Text Centered on Stamp (Horizontally and Vertically))
    // Font Size for the stamped exhibit number
    let font_size_mid_num = 20.0; // Large size for readability

    let mid_zone_center = (line1_y + line2_y) / 2.0;
    let mid_num_y = mid_zone_center - (font_size_mid_num * 0.35);
    let mid_num_x = center_text_offset(exhibit_num, font_size_mid_num, x, w);

    // BOTTOM ZONE: footer (can be a file name or other identifying text)
    let font_size_bot = 6.0;
    let bot_zone_center = (line2_y + y) / 2.0;
    let bot_y = bot_zone_center - (font_size_bot * 0.35);
    let bot_x = center_text_offset(footer, font_size_bot, x, w);

    // -------------------------------------------------------------
    // 5. Draw Vector Text
    // -------------------------------------------------------------
    ops.push(Operation::new("BT", vec![]));
    ops.push(Operation::new(
        "rg",
        vec![0.0.into(), 0.0.into(), 0.0.into()],
    ));

    // 1. Top Zone (Header) (Exhibit Type)
    ops.push(Operation::new(
        "Tf",
        vec![
            Object::Name(font_alias.as_bytes().to_vec()),
            font_size_top.into(),
        ],
    ));
    ops.push(Operation::new("Td", vec![top_x.into(), top_y.into()]));
    ops.push(Operation::new("Tj", vec![Object::string_literal(header)]));

    // 2. Middle Zone (Exhibit Number)
    ops.push(Operation::new(
        "Tf",
        vec![
            Object::Name(font_alias.as_bytes().to_vec()),
            font_size_mid_num.into(),
        ],
    ));
    ops.push(Operation::new(
        "Td",
        vec![(mid_num_x - top_x).into(), (mid_num_y - top_y).into()],
    ));
    ops.push(Operation::new(
        "Tj",
        vec![Object::string_literal(exhibit_num)],
    ));

    // 3. Bottom Zone (Footer)
    ops.push(Operation::new(
        "Tf",
        vec![
            Object::Name(font_alias.as_bytes().to_vec()),
            font_size_bot.into(),
        ],
    ));
    ops.push(Operation::new(
        "Td",
        vec![(bot_x - mid_num_x).into(), (bot_y - mid_num_y).into()],
    ));
    ops.push(Operation::new("Tj", vec![Object::string_literal(footer)]));

    ops.push(Operation::new("ET", vec![]));
    ops.push(Operation::new("Q", vec![]));

    // 6. Save content stream back to PDF
    let stamp_stream = Stream::new(dictionary! {}, Content { operations: ops }.encode()?);
    let stamp_stream_id = doc.add_object(stamp_stream);

    // Isolate the original content so unbalanced `cm`/`q` in it can't transform the stamp.
    let save_id = doc.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
    let restore_id = doc.add_object(Stream::new(dictionary! {}, b"\nQ\n".to_vec()));

    let page_dict = doc.get_object_mut(page_id)?.as_dict_mut()?;
    let mut contents = match page_dict.get(b"Contents") {
        Ok(Object::Array(existing)) => existing.clone(),
        Ok(Object::Reference(id)) => vec![Object::Reference(*id)],
        _ => vec![],
    };
    contents.insert(0, Object::Reference(save_id));
    contents.push(Object::Reference(restore_id));
    contents.push(Object::Reference(stamp_stream_id));
    page_dict.set("Contents", contents);

    Ok(())
}

/// Appends a vector rounded rectangle path using Bézier Curves (c operator)
fn append_rounded_rect_path(ops: &mut Vec<Operation>, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let k = 0.552284749831 * r; // Magic constant for drawing circular arcs with cubic Béziers

    ops.push(Operation::new("m", vec![(x + r).into(), y.into()]));
    ops.push(Operation::new("l", vec![(x + w - r).into(), y.into()]));
    ops.push(Operation::new(
        "c",
        vec![
            (x + w - r + k).into(),
            y.into(),
            (x + w).into(),
            (y + r - k).into(),
            (x + w).into(),
            (y + r).into(),
        ],
    ));
    ops.push(Operation::new(
        "l",
        vec![(x + w).into(), (y + h - r).into()],
    ));
    ops.push(Operation::new(
        "c",
        vec![
            (x + w).into(),
            (y + h - r + k).into(),
            (x + w - r + k).into(),
            (y + h).into(),
            (x + w - r).into(),
            (y + h).into(),
        ],
    ));
    ops.push(Operation::new("l", vec![(x + r).into(), (y + h).into()]));
    ops.push(Operation::new(
        "c",
        vec![
            (x + r - k).into(),
            (y + h).into(),
            x.into(),
            (y + h - r + k).into(),
            x.into(),
            (y + h - r).into(),
        ],
    ));
    ops.push(Operation::new("l", vec![x.into(), (y + r).into()]));
    ops.push(Operation::new(
        "c",
        vec![
            x.into(),
            (y + r - k).into(),
            (x + r - k).into(),
            y.into(),
            (x + r).into(),
            y.into(),
        ],
    ));
    ops.push(Operation::new("h", vec![])); // Close subpath
}

// /// Approximate horizontal text centering calculation for Standard Helvetica
// fn center_text_offset(text: &str, font_size: f64, box_x: f64, box_w: f64) -> f64 {
//     let avg_char_width = font_size * 0.55;
//     let approx_text_width = text.len() as f64 * avg_char_width;
//     box_x + ((box_w - approx_text_width) / 2.0).max(5.0)
// }

/// Calculates horizontal centering using Helvetica-Bold's standard glyph widths.
fn center_text_offset(text: &str, font_size: f64, box_x: f64, box_w: f64) -> f64 {
    let text_width = measure_helvetica_bold_width(text, font_size);
    box_x + (box_w - text_width) / 2.0
}

/// Returns the width of a string in points using Helvetica-Bold AFM widths.
fn measure_helvetica_bold_width(text: &str, font_size: f64) -> f64 {
    let total_units: u32 = text
        .chars()
        .map(|c| match c {
            ' ' => 278,
            '!' => 333,
            '"' => 474,
            '#' | '$' => 556,
            '%' => 889,
            '&' => 722,
            '\'' => 238,
            '(' | ')' => 333,
            '*' => 389,
            '+' => 584,
            ',' | '.' => 278,
            '-' => 333,
            '/' => 278,
            '0'..='9' => 556,
            ':' | ';' => 333,
            '<' | '=' | '>' => 584,
            '?' => 611,
            '@' => 975,
            'A' | 'C' | 'D' | 'U' => 722,
            'B' | 'E' | 'P' | 'X' | 'Y' => 667,
            'F' | 'S' => 611,
            'G' | 'O' | 'Q' => 778,
            'H' | 'N' | 'R' => 722,
            'I' => 278,
            'J' => 556,
            'K' => 722,
            'L' => 611,
            'M' => 833,
            'T' | 'Z' => 611,
            'V' => 667,
            'W' => 944,
            '[' | ']' => 333,
            '\\' => 278,
            '^' => 584,
            '_' => 556,
            '`' => 333,
            'a' | 'c' | 'e' | 's' | 'x' => 556,
            'b' | 'n' | 'o' | 'p' | 'q' | 'u' => 611,
            'd' | 'g' | 'h' => 611,
            'f' | 't' => 333,
            'i' | 'j' | 'l' => 278,
            'k' => 556,
            'm' => 889,
            'r' => 389,
            'v' | 'y' => 556,
            'w' => 778,
            'z' => 500,
            '{' | '}' => 389,
            '|' => 280,
            '~' => 584,
            _ => 600,
        })
        .sum();

    // Standard PDF glyph widths are defined in 1/1000ths of a text unit
    (total_units as f64 / 1000.0) * font_size
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use lopdf::{Document, Object, Stream, dictionary};
    use std::fs;
    use std::path::Path;

    #[test]
    fn output_argument_is_optional_and_defaults_to_stamped_input_name() {
        let args =
            Args::try_parse_from(["exhibit-stamp", "--input", "records/evidence.pdf"]).unwrap();

        assert_eq!(args.output, None);
        assert_eq!(
            stamped_output_path(&args.input),
            PathBuf::from("records/evidence_stamped.pdf")
        );

        let args = Args::try_parse_from([
            "exhibit-stamp",
            "--input",
            "records/evidence.pdf",
            "--output",
            "stamped.pdf",
        ])
        .unwrap();
        assert_eq!(args.output, Some(PathBuf::from("stamped.pdf")));
    }

    /// Helper to create a dummy PDF in memory with standard 12pt body text and 1-inch margins
    fn create_dummy_pdf() -> Document {
        let mut doc = Document::with_version("1.5");

        // 1. Create Base Pages Dictionary
        let pages_id = doc.add_object(dictionary! {
            "Type" => "Pages",
            "Count" => 1i64,
        });

        // 2. Create Page Dictionary
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], // Letter: 8.5" x 11" (72 pt/inch)
        });

        if let Ok(pages_dict) = doc.get_object_mut(pages_id).and_then(|o| o.as_dict_mut()) {
            pages_dict.set("Kids", vec![Object::Reference(page_id)]);
        }

        // 3. Register Standard Helvetica Font
        let font_dict = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        };
        let font_id = doc.add_object(font_dict);

        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! {
                "BodyFont" => font_id,
            },
        });

        // 4. Generate Body Text with 1-Inch (72pt) Margins
        let margin_left = 72.0;
        let margin_top = 792.0 - 72.0; // 720.0 pt baseline start
        let font_size = 12.0;
        let leading = 16.0; // Line spacing for 12pt text

        let paragraphs = vec![
            "IN THE CIRCUIT COURT OF THE FIRST JUDICIAL CIRCUIT",
            "IN AND FOR COOK COUNTY, STATE OF ILLINOIS",
            "",
            "MEMORANDUM OF LAW IN SUPPORT OF MOTION",
            "",
            "The defendant respectfully moves this Honorable Court for an order granting the relief requested herein. As established in the record, all relevant evidence was collected in strict accordance with statutory requirements and applicable constitutional protections.",
            "",
            "Furthermore, administrative review demonstrates that the documentation provided satisfies all procedural standards required for admissibility under the Rules of Evidence.",
            "IN THE CIRCUIT COURT OF THE FIRST JUDICIAL CIRCUIT",
            "IN AND FOR COOK COUNTY, STATE OF ILLINOIS",
            "IN THE CIRCUIT COURT OF THE FIRST JUDICIAL CIRCUIT",
            "IN AND FOR COOK COUNTY, STATE OF ILLINOIS",
            "IN THE CIRCUIT COURT OF THE FIRST JUDICIAL CIRCUIT",
            "IN AND FOR COOK COUNTY, STATE OF ILLINOIS",
            "IN THE CIRCUIT COURT OF THE FIRST JUDICIAL CIRCUIT",
            "IN AND FOR COOK COUNTY, STATE OF ILLINOIS",
            "IN THE CIRCUIT COURT OF THE FIRST JUDICIAL CIRCUIT",
            "IN AND FOR COOK COUNTY, STATE OF ILLINOIS",
            "IN THE CIRCUIT COURT OF THE FIRST JUDICIAL CIRCUIT",
            "IN AND FOR COOK COUNTY, STATE OF ILLINOIS",
            "IN THE CIRCUIT COURT OF THE FIRST JUDICIAL CIRCUIT",
            "IN AND FOR COOK COUNTY, STATE OF ILLINOIS",
            "IN THE CIRCUIT COURT OF THE FIRST JUDICIAL CIRCUIT",
            "IN AND FOR COOK COUNTY, STATE OF ILLINOIS",
        ];

        let mut ops = Vec::new();
        ops.push(Operation::new("BT", vec![])); // Begin Text
        ops.push(Operation::new(
            "Tf",
            vec![Object::Name(b"BodyFont".to_vec()), font_size.into()],
        ));
        ops.push(Operation::new("TL", vec![leading.into()])); // Set Line Leading
        ops.push(Operation::new(
            "Td",
            vec![margin_left.into(), margin_top.into()],
        ));

        for (i, line) in paragraphs.iter().enumerate() {
            if line.is_empty() {
                // Move down an extra line for paragraph spacing
                ops.push(Operation::new("T*", vec![]));
            } else {
                ops.push(Operation::new("Tj", vec![Object::string_literal(*line)]));
                if i < paragraphs.len() - 1 {
                    ops.push(Operation::new("T*", vec![])); // Move to next line
                }
            }
        }

        ops.push(Operation::new("ET", vec![])); // End Text

        // 5. Attach Content Stream and Resources to Page
        let content_stream = Stream::new(
            dictionary! {},
            Content { operations: ops }.encode().unwrap(),
        );
        let content_id = doc.add_object(content_stream);

        if let Ok(page_dict) = doc.get_object_mut(page_id).and_then(|o| o.as_dict_mut()) {
            page_dict.set("Resources", resources_id);
            page_dict.set("Contents", vec![Object::Reference(content_id)]);
        }

        // 6. Set Catalog Root
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });

        doc.trailer.set("Root", catalog_id);
        doc
    }

    #[test]
    fn test_stamp_vector_exhibit_structure() {
        let mut doc = create_dummy_pdf();
        let initial_object_count = doc.objects.len();

        // Apply vector stamp to page 1
        let result = stamp_vector_exhibit(
            &mut doc,
            1,
            &Some("PRELIMINARY HEARING".to_owned()),
            "EXHIBIT 14",
            "CR-2026-00892",
            &ExhibitType::Defense,
            10.0,
            0.0,
            220.0,
            130.0,
        );

        assert!(result.is_ok(), "Stamping failed: {:?}", result.err());

        // Verify that new PDF objects were created (Font, Stream, etc.)
        assert!(
            doc.objects.len() > initial_object_count,
            "No new PDF objects were added to the document"
        );

        // Verify page 1 now contains a /Contents reference
        let pages = doc.get_pages();
        let page_id = *pages.get(&1).unwrap();
        let page_dict = doc.get_object(page_id).unwrap().as_dict().unwrap();

        assert!(
            page_dict.has(b"Contents"),
            "Page object is missing the /Contents key"
        );
    }

    #[test]
    fn test_stamp_vector_ex() {
        // Open PDF document for testing
        // Open the document "test_2.pdf"
        let mut test_2 = lopdf::Document::load("test_2.pdf").unwrap();

        let initial_object_count = test_2.objects.len();

        // Apply vector stamp to page 1

        let result = stamp_vector_exhibit(
            &mut test_2,
            1,
            &Some("Defense Exhibit".to_owned()),
            "R14",
            "A File Name",
            &ExhibitType::Defense,
            10.0,
            0.0,
            80.0,
            50.0,
        );
        assert!(result.is_ok(), "Stamping failed: {:?}", result.err());

        // Verify that new PDF objects were created (Font, Stream, etc.)
        assert!(
            test_2.objects.len() > initial_object_count,
            "No new PDF objects were added to the document"
        );

        // Save the PDF to the project directory for inspection
        test_2.save("test_2_stamped.pdf").unwrap();
    }

    #[test]
    fn test_generate_visual_pdf_outputs() {
        let output_dir = Path::new("target/test_output");
        fs::create_dir_all(output_dir).unwrap();

        // 1. Generate Defense Exhibit PDF (Light Blue)
        let mut doc_defense = create_dummy_pdf();
        stamp_vector_exhibit(
            &mut doc_defense,
            1,
            &None,
            "R61",
            "File Name",
            &ExhibitType::Defense,
            10.0,
            0.0,
            90.0,
            60.0,
        )
        .expect("Failed to stamp Defense exhibit");

        let defense_path = output_dir.join("test_defense.pdf");
        doc_defense.save(&defense_path).unwrap();
        assert!(defense_path.exists());

        // 2. Generate Government Exhibit PDF (Light Yellow)
        let mut doc_govt = create_dummy_pdf();
        stamp_vector_exhibit(
            &mut doc_govt,
            1,
            &Some("EVIDENTIARY HEARING".to_owned()),
            "101",
            "2026-CV1-99182",
            &ExhibitType::Government,
            10.0,
            0.0,
            90.0,
            60.0,
        )
        .expect("Failed to stamp Government exhibit");

        let govt_path = output_dir.join("test_government.pdf");
        doc_govt.save(&govt_path).unwrap();
        assert!(govt_path.exists());

        println!(
            "\nVisual PDFs saved to:\n  - {}\n  - {}",
            defense_path.canonicalize().unwrap().display(),
            govt_path.canonicalize().unwrap().display()
        );
    }
}
