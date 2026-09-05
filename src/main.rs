use clap::{Parser, ValueEnum};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use std::path::PathBuf;

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
    output: PathBuf,

    #[arg(short, long, default_value_t = 1)]
    page: u32,

    #[arg(value_enum, short = 't', long, default_value_t = ExhibitType::Defense)]
    exhibit_type: ExhibitType,

    #[arg(short, long, default_value = "EXHIBIT 14")]
    exhibit: String,

    #[arg(long, default_value = "SUPERIOR COURT OF CALIFORNIA")]
    title: String,

    #[arg(long, default_value = "CR-2026-00892")]
    case_no: String,

    /// Position X in PDF points (1/72 inch)
    #[arg(short, long, default_value_t = 612.0 - 90.0)]
    x: f64,

    /// Position Y in PDF points (1/72 inch)
    #[arg(short, long, default_value_t = 50.0)]
    y: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut doc = Document::load(&args.input)?;

    stamp_vector_exhibit(
        &mut doc,
        args.page,
        &args.title,
        &args.exhibit,
        &args.case_no,
        &args.exhibit_type,
        args.x,
        args.y,
        220.0, // width in points
        130.0, // height in points
    )?;

    doc.save(&args.output)?;
    println!("Vector exhibit stamp applied cleanly to {:?}", args.output);
    Ok(())
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
    hearing_type: &str,
    exhibit: &str,
    case_no: &str,
    exhibit_type: &ExhibitType,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    let pages = doc.get_pages();
    let page_id = *pages.get(&page_num).ok_or("Page index out of bounds")?;

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
    let line1_y = y + (h * 0.80);
    let line2_y = y + (h * 0.18);

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

    // TOP ZONE: hearing_type
    let font_size_top = 7.0;
    let top_zone_center = y + (h * 0.90);
    let top_y = top_zone_center - (font_size_top * 0.35);
    let top_x = center_text_offset(hearing_type, font_size_top, x, w);

    // MIDDLE ZONE: Stacked Exhibit Label + Exhibit Number
    // Determine label based on ExhibitType enum
    let exhibit_label = match exhibit_type {
        ExhibitType::Defense => "DEFENSE EXHIBIT",
        ExhibitType::Government => "GOVERNMENT EXHIBIT",
    };

    let font_size_mid_label = 9.0; // Matching hearing_type / case_no size
    let font_size_mid_num = 20.0; // Large size for readability
    let line_spacing = 7.0; // Gap between the two lines

    // Total vertical span of the 2-line text block (cap-heights + gap)
    let label_cap_h = font_size_mid_label * 0.70;
    let num_cap_h = font_size_mid_num * 0.70;
    let total_block_h = label_cap_h + num_cap_h + line_spacing;

    // Midpoint of the middle zone (spans between 0.18h and 0.80h)
    let mid_zone_center = y + (h * 0.49);

    // Calculate top of the text block to center both lines together
    let block_top = mid_zone_center + (total_block_h / 2.0);

    // Baselines for the two middle lines
    let mid_label_y = block_top - label_cap_h;
    let mid_num_y = mid_label_y - line_spacing - num_cap_h;

    let mid_label_x = center_text_offset(exhibit_label, font_size_mid_label, x, w);
    let mid_num_x = center_text_offset(exhibit, font_size_mid_num, x, w);

    // BOTTOM ZONE: case_no
    let font_size_bot = 7.0;
    let bot_zone_center = y + (h * 0.09);
    let bot_y = bot_zone_center - (font_size_bot * 0.35);
    let bot_x = center_text_offset(case_no, font_size_bot, x, w);

    // -------------------------------------------------------------
    // 5. Draw Vector Text
    // -------------------------------------------------------------
    ops.push(Operation::new("BT", vec![]));
    ops.push(Operation::new(
        "rg",
        vec![0.0.into(), 0.0.into(), 0.0.into()],
    ));

    // 1. Top Zone (Hearing Type)
    ops.push(Operation::new(
        "Tf",
        vec![
            Object::Name(font_alias.as_bytes().to_vec()),
            font_size_top.into(),
        ],
    ));
    ops.push(Operation::new("Td", vec![top_x.into(), top_y.into()]));
    ops.push(Operation::new(
        "Tj",
        vec![Object::string_literal(hearing_type)],
    ));

    // 2. Middle Zone - Line 1 (Exhibit Type Label)
    ops.push(Operation::new(
        "Tf",
        vec![
            Object::Name(font_alias.as_bytes().to_vec()),
            font_size_mid_label.into(),
        ],
    ));
    ops.push(Operation::new(
        "Td",
        vec![(mid_label_x - top_x).into(), (mid_label_y - top_y).into()],
    ));
    ops.push(Operation::new(
        "Tj",
        vec![Object::string_literal(exhibit_label)],
    ));

    // 3. Middle Zone - Line 2 (Exhibit Number)
    ops.push(Operation::new(
        "Tf",
        vec![
            Object::Name(font_alias.as_bytes().to_vec()),
            font_size_mid_num.into(),
        ],
    ));
    ops.push(Operation::new(
        "Td",
        vec![
            (mid_num_x - mid_label_x).into(),
            (mid_num_y - mid_label_y).into(),
        ],
    ));
    ops.push(Operation::new("Tj", vec![Object::string_literal(exhibit)]));

    // 4. Bottom Zone (Case Number)
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
    ops.push(Operation::new("Tj", vec![Object::string_literal(case_no)]));

    ops.push(Operation::new("ET", vec![]));
    ops.push(Operation::new("Q", vec![]));

    // 6. Save content stream back to PDF
    let stamp_stream = Stream::new(dictionary! {}, Content { operations: ops }.encode()?);
    let stamp_stream_id = doc.add_object(stamp_stream);

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

/// Calculates exact horizontal centering for Helvetica-Bold text in PDF points
fn center_text_offset(text: &str, font_size: f64, box_x: f64, box_w: f64) -> f64 {
    let text_width = measure_helvetica_bold_width(text, font_size);
    box_x + ((box_w - text_width) / 2.0).max(0.0)
}

/// Returns the exact width of a string in points for Helvetica-Bold
fn measure_helvetica_bold_width(text: &str, font_size: f64) -> f64 {
    let total_units: u32 = text
        .chars()
        .map(|c| match c {
            // Numbers
            '0'..='9' => 556,
            // Uppercase Letters
            'A' | 'B' | 'E' | 'F' | 'K' | 'P' | 'R' => 667,
            'C' | 'D' | 'G' | 'H' | 'N' | 'O' | 'Q' | 'U' => 722,
            'I' => 278,
            'J' => 500,
            'L' => 556,
            'M' => 833,
            'S' => 611,
            'T' => 611,
            'V' | 'Y' => 667,
            'W' => 944,
            'X' => 667,
            'Z' => 611,
            // Common Punctuation & Symbols
            ' ' => 278,
            '-' => 333,
            '.' => 278,
            ':' => 333,
            '#' => 556,
            '/' => 278,
            // Fallback for unmapped characters
            _ => 600,
        })
        .sum();

    // Standard PDF glyph widths are defined in 1/1000ths of a text unit
    (total_units as f64 / 1000.0) * font_size
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Document, Object, Stream, dictionary};
    use std::fs;
    use std::path::Path;

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
            "PRELIMINARY HEARING",
            "EXHIBIT 14",
            "CR-2026-00892",
            &ExhibitType::Defense,
            350.0,
            50.0,
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
    fn test_generate_visual_pdf_outputs() {
        let output_dir = Path::new("target/test_output");
        fs::create_dir_all(output_dir).unwrap();

        // 1. Generate Defense Exhibit PDF (Light Blue)
        let mut doc_defense = create_dummy_pdf();
        stamp_vector_exhibit(
            &mut doc_defense,
            1,
            "TRIAL",
            "A",
            "2026-CV1-99182",
            &ExhibitType::Defense,
            500.0,
            50.0,
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
            "EVIDENTIARY HEARING",
            "101",
            "2026-CV1-99182",
            &ExhibitType::Government,
            00.0, // Placed at bottom-left corner
            00.0,
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
