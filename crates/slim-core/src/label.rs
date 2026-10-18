//! Label and QR code generation for items, Gridfinity bins, and locations.

use qrcode::QrCode;

use crate::error::{CoreError, CoreResult};
use crate::model::gridfinity::{GRIDFINITY_PITCH_MM, GridfinityBin};
use crate::model::item::Item;
use crate::model::location::Location;
use crate::model::scan::{bin_uri, item_uri, location_uri};

/// Generate a high-contrast compact Unicode half-block QR code for terminal rendering.
pub fn generate_terminal_qr(payload: &str) -> CoreResult<String> {
    let code = QrCode::new(payload.as_bytes())
        .map_err(|e| CoreError::InvalidInput(format!("failed to encode QR code: {}", e)))?;
    let image = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Dark)
        .light_color(qrcode::render::unicode::Dense1x2::Light)
        .build();
    Ok(image)
}

/// Generate a standalone SVG vector image string of the QR code.
pub fn generate_svg_qr(payload: &str) -> CoreResult<String> {
    let code = QrCode::new(payload.as_bytes())
        .map_err(|e| CoreError::InvalidInput(format!("failed to encode QR code: {}", e)))?;
    let svg = code
        .render()
        .min_dimensions(200, 200)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok(svg)
}

/// Format a complete printable terminal label for an inventory catalog item.
pub fn generate_item_label(item: &Item) -> CoreResult<String> {
    let payload = item_uri(&item.id.to_string());
    let qr_block = generate_terminal_qr(&payload)?;

    let mut out = String::new();
    out.push_str("┌────────────────────────────────────────────────────────┐\n");
    out.push_str("│                SLIM INVENTORY ITEM LABEL               │\n");
    out.push_str("├────────────────────────────────────────────────────────┤\n");
    out.push_str(&format!(
        "│ SKU:  {:<48} │\n",
        item.sku.as_deref().unwrap_or("(No SKU assigned)")
    ));
    out.push_str(&format!("│ NAME: {:<48} │\n", item.name));
    out.push_str(&format!("│ UNIT: {:<48} │\n", item.unit));
    out.push_str(&format!("│ URI:  {:<48} │\n", payload));
    out.push_str("├────────────────────────────────────────────────────────┤\n");
    out.push_str("│                  SCAN TO IDENTIFY / TRANSACT           │\n");
    out.push_str("└────────────────────────────────────────────────────────┘\n\n");
    out.push_str(&qr_block);
    out.push('\n');

    Ok(out)
}

/// Format a complete printable terminal label for a physical Gridfinity bin.
pub fn generate_bin_label(bin: &GridfinityBin, parent_name: Option<&str>) -> CoreResult<String> {
    let payload = bin_uri(&bin.bin_code);
    let qr_block = generate_terminal_qr(&payload)?;

    let mut out = String::new();
    out.push_str("┌────────────────────────────────────────────────────────┐\n");
    out.push_str("│                 GRIDFINITY STORAGE BIN                 │\n");
    out.push_str("├────────────────────────────────────────────────────────┤\n");
    out.push_str(&format!("│ BIN CODE:     {:<40} │\n", bin.bin_code));
    out.push_str(&format!(
        "│ GRID UNITS:   {:<40} │\n",
        format!(
            "{}W x {}D x {}H",
            bin.width_units, bin.depth_units, bin.height_units
        )
    ));
    out.push_str(&format!(
        "│ PHYSICAL:     {:<40} │\n",
        format!(
            "{:.1} x {:.1} x {:.1} mm",
            bin.width_mm(GRIDFINITY_PITCH_MM),
            bin.depth_mm(GRIDFINITY_PITCH_MM),
            bin.height_mm()
        )
    ));
    out.push_str(&format!(
        "│ COMPARTMENTS: {:<40} │\n",
        bin.compartment_count
    ));
    if let Some(parent) = parent_name {
        out.push_str(&format!("│ MOUNTED IN:   {:<40} │\n", parent));
    }
    out.push_str(&format!("│ URI:          {:<40} │\n", payload));
    out.push_str("├────────────────────────────────────────────────────────┤\n");
    out.push_str("│             SCAN TO LOCATE / CHECK-IN / CHECK-OUT      │\n");
    out.push_str("└────────────────────────────────────────────────────────┘\n\n");
    out.push_str(&qr_block);
    out.push('\n');

    Ok(out)
}

/// Format a complete printable terminal label for a generic storage location.
pub fn generate_location_label(location: &Location) -> CoreResult<String> {
    let payload = location_uri(&location.name);
    let qr_block = generate_terminal_qr(&payload)?;

    let mut out = String::new();
    out.push_str("┌────────────────────────────────────────────────────────┐\n");
    out.push_str("│                 SLIM STORAGE LOCATION                  │\n");
    out.push_str("├────────────────────────────────────────────────────────┤\n");
    out.push_str(&format!("│ NAME: {:<48} │\n", location.name));
    out.push_str(&format!("│ KIND: {:<48} │\n", location.kind));
    out.push_str(&format!("│ ID:   {:<48} │\n", location.id));
    out.push_str(&format!("│ URI:  {:<48} │\n", payload));
    out.push_str("├────────────────────────────────────────────────────────┤\n");
    out.push_str("│                 SCAN TO SELECT LOCATION                │\n");
    out.push_str("└────────────────────────────────────────────────────────┘\n\n");
    out.push_str(&qr_block);
    out.push('\n');

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::gridfinity::GridfinityBinId;
    use crate::model::item::ItemId;
    use crate::model::location::LocationId;
    use time::OffsetDateTime;

    #[test]
    fn test_generate_terminal_qr_produces_non_empty_half_blocks() {
        let qr = generate_terminal_qr("slim:item:M3X10").unwrap();
        assert!(!qr.is_empty());
        // Dense1x2 render uses Unicode characters
        assert!(qr.lines().count() >= 10);
    }

    #[test]
    fn test_generate_svg_qr_produces_valid_svg_string() {
        let svg = generate_svg_qr("slim:bin:BIN-001").unwrap();
        assert!(svg.starts_with("<?xml") || svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }

    #[test]
    fn test_generate_item_label() {
        let item = Item {
            id: ItemId::new(),
            sku: Some("RES-10K".to_string()),
            name: "10k Resistor".to_string(),
            unit: "each".to_string(),
            description: None,
            metadata: None,
            archived_at: None,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        };
        let label = generate_item_label(&item).unwrap();
        assert!(label.contains("RES-10K"));
        assert!(label.contains("10k Resistor"));
        assert!(label.contains(&format!("slim:item:{}", item.id)));
    }

    #[test]
    fn test_generate_bin_label() {
        let bin = GridfinityBin {
            id: GridfinityBinId::new(),
            location_id: LocationId::new(),
            bin_code: "B-2x2".to_string(),
            width_units: 2,
            depth_units: 2,
            height_units: 6,
            compartment_count: 2,
            created_at: OffsetDateTime::now_utc(),
        };
        let label = generate_bin_label(&bin, Some("Top Drawer")).unwrap();
        assert!(label.contains("B-2x2"));
        assert!(label.contains("2W x 2D x 6H"));
        assert!(label.contains("Top Drawer"));
        assert!(label.contains("slim:bin:B-2x2"));
    }
}
