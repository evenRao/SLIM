//! Barcode, QR code, and scanner target domain models.

use serde::{Deserialize, Serialize};

use super::gridfinity::GridfinityBin;
use super::item::Item;
use super::location::Location;

/// Standard SLIM URI scheme prefixes for encoded QR codes.
pub const URI_PREFIX_ITEM: &str = "slim:item:";
pub const URI_PREFIX_BIN: &str = "slim:bin:";
pub const URI_PREFIX_LOC: &str = "slim:loc:";

/// Helper to generate a standardized SLIM URI for an item.
pub fn item_uri(sku_or_id: &str) -> String {
    format!("{}{}", URI_PREFIX_ITEM, sku_or_id.trim())
}

/// Helper to generate a standardized SLIM URI for a Gridfinity bin.
pub fn bin_uri(bin_code_or_id: &str) -> String {
    format!("{}{}", URI_PREFIX_BIN, bin_code_or_id.trim())
}

/// Helper to generate a standardized SLIM URI for a location.
pub fn location_uri(name_or_id: &str) -> String {
    format!("{}{}", URI_PREFIX_LOC, name_or_id.trim())
}

/// Parsed SLIM URI prefix and identifier value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedUri<'a> {
    Item(&'a str),
    Bin(&'a str),
    Location(&'a str),
    Raw(&'a str),
}

impl<'a> ParsedUri<'a> {
    pub fn parse(input: &'a str) -> Self {
        let trimmed = input.trim();
        if let Some(rest) = trimmed.strip_prefix(URI_PREFIX_ITEM) {
            ParsedUri::Item(rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix(URI_PREFIX_BIN) {
            ParsedUri::Bin(rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix(URI_PREFIX_LOC) {
            ParsedUri::Location(rest.trim())
        } else {
            ParsedUri::Raw(trimmed)
        }
    }
}

/// Resolved target of a scanned barcode or QR code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "target_type", content = "target")]
pub enum ScanTarget {
    /// The scanned code resolved to a catalog item.
    Item(Item),
    /// The scanned code resolved to a Gridfinity bin (and its backing location).
    Bin {
        bin: GridfinityBin,
        location: Location,
    },
    /// The scanned code resolved to a generic storage location.
    Location(Location),
    /// The scanned code was not found in the database.
    Unregistered { scanned_code: String },
}

impl ScanTarget {
    pub fn display_label(&self) -> String {
        match self {
            ScanTarget::Item(item) => {
                let sku_str = item.sku.as_deref().unwrap_or("NO-SKU");
                format!("Item: {} ({})", item.name, sku_str)
            }
            ScanTarget::Bin { bin, .. } => {
                format!(
                    "Gridfinity Bin: {} ({}x{}x{}U)",
                    bin.bin_code, bin.width_units, bin.depth_units, bin.height_units
                )
            }
            ScanTarget::Location(loc) => {
                format!("Location: {} [{}]", loc.name, loc.kind)
            }
            ScanTarget::Unregistered { scanned_code } => {
                format!("Unregistered Code: '{}'", scanned_code)
            }
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            ScanTarget::Item(_) => "item",
            ScanTarget::Bin { .. } => "bin",
            ScanTarget::Location(_) => "location",
            ScanTarget::Unregistered { .. } => "unregistered",
        }
    }

    pub fn is_item(&self) -> bool {
        matches!(self, ScanTarget::Item(_))
    }

    pub fn is_bin(&self) -> bool {
        matches!(self, ScanTarget::Bin { .. })
    }

    pub fn is_location(&self) -> bool {
        matches!(self, ScanTarget::Location(_))
    }

    pub fn is_registered(&self) -> bool {
        !matches!(self, ScanTarget::Unregistered { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uri_generation() {
        assert_eq!(item_uri("M3X10"), "slim:item:M3X10");
        assert_eq!(bin_uri("BIN-001"), "slim:bin:BIN-001");
        assert_eq!(location_uri("Drawer 1"), "slim:loc:Drawer 1");
    }

    #[test]
    fn test_uri_parsing() {
        assert_eq!(
            ParsedUri::parse("slim:item:M3X10"),
            ParsedUri::Item("M3X10")
        );
        assert_eq!(ParsedUri::parse("slim:bin:B-42"), ParsedUri::Bin("B-42"));
        assert_eq!(
            ParsedUri::parse("slim:loc:Shelf A"),
            ParsedUri::Location("Shelf A")
        );
        assert_eq!(
            ParsedUri::parse("01234567890"),
            ParsedUri::Raw("01234567890")
        );
    }

    #[test]
    fn test_scan_target_serde_round_trip() {
        let target = ScanTarget::Unregistered {
            scanned_code: "UNKNOWN-123".to_string(),
        };
        let json = serde_json::to_string(&target).unwrap();
        let deserialized: ScanTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(target, deserialized);
        assert!(!target.is_registered());
        assert_eq!(target.type_name(), "unregistered");
    }
}
