//! Gridfinity domain models, geometry invariants, and coordinate math.
//!
//! Standard Gridfinity pitch is 42.0 mm per grid unit.
//! Height units are 7.0 mm increments.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use time::OffsetDateTime;
use uuid::Uuid;

use super::location::LocationId;
use crate::error::{CoreError, CoreResult};

/// Standard Gridfinity horizontal pitch in millimeters.
pub const GRIDFINITY_PITCH_MM: f64 = 42.0;

/// Standard Gridfinity vertical height unit increment in millimeters.
pub const GRIDFINITY_HEIGHT_INCREMENT_MM: f64 = 7.0;

/// Unique identifier for a Gridfinity baseplate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GridBaseId(pub Uuid);

impl GridBaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for GridBaseId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for GridBaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for GridBaseId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Unique identifier for a Gridfinity bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GridfinityBinId(pub Uuid);

impl GridfinityBinId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for GridfinityBinId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for GridfinityBinId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for GridfinityBinId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Unique identifier for a bin placement record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlacementId(pub Uuid);

impl PlacementId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PlacementId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PlacementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PlacementId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Orientation / rotation of a bin on a baseplate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Rotation {
    #[default]
    Rot0 = 0,
    Rot90 = 90,
}

impl fmt::Display for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rotation::Rot0 => write!(f, "0"),
            Rotation::Rot90 => write!(f, "90"),
        }
    }
}

impl FromStr for Rotation {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "0" | "0deg" | "rot0" => Ok(Rotation::Rot0),
            "90" | "90deg" | "rot90" => Ok(Rotation::Rot90),
            _ => Err(CoreError::InvalidInput(format!(
                "invalid rotation '{s}': expected 0 or 90"
            ))),
        }
    }
}

/// A physical baseplate mounted in a location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridBase {
    pub id: GridBaseId,
    pub location_id: LocationId,
    pub code: String,
    pub cols: u32,
    pub rows: u32,
    pub pitch_mm: f64,
    pub created_at: OffsetDateTime,
}

impl GridBase {
    /// Calculate the physical dimensions in millimeters.
    pub fn width_mm(&self) -> f64 {
        self.cols as f64 * self.pitch_mm
    }

    pub fn depth_mm(&self) -> f64 {
        self.rows as f64 * self.pitch_mm
    }
}

/// Parameters for creating a new baseplate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewGridBase {
    pub location_id: LocationId,
    pub code: String,
    pub cols: u32,
    pub rows: u32,
    pub pitch_mm: Option<f64>,
}

impl NewGridBase {
    pub fn validate(&self) -> CoreResult<()> {
        if self.code.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "base code must not be empty".to_string(),
            ));
        }
        if self.cols == 0 || self.rows == 0 {
            return Err(CoreError::InvalidInput(
                "base dimensions must be greater than zero".to_string(),
            ));
        }
        if self.cols > 100 || self.rows > 100 {
            return Err(CoreError::InvalidInput(
                "A baseplate supports at most 100 columns and 100 rows".into(),
            ));
        }
        if let Some(pitch) = self.pitch_mm
            && (!pitch.is_finite() || pitch <= 0.0)
        {
            return Err(CoreError::InvalidInput(
                "pitch must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

/// A movable physical Gridfinity bin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridfinityBin {
    pub id: GridfinityBinId,
    pub location_id: LocationId,
    pub bin_code: String,
    pub width_units: u32,
    pub depth_units: u32,
    pub height_units: u32,
    pub compartment_count: u32,
    pub created_at: OffsetDateTime,
}

impl GridfinityBin {
    /// Calculate effective dimensions given a rotation.
    pub fn effective_dimensions(&self, rotation: Rotation) -> (u32, u32) {
        match rotation {
            Rotation::Rot0 => (self.width_units, self.depth_units),
            Rotation::Rot90 => (self.depth_units, self.width_units),
        }
    }

    /// Calculate physical dimensions in millimeters.
    pub fn width_mm(&self, pitch_mm: f64) -> f64 {
        self.width_units as f64 * pitch_mm
    }

    pub fn depth_mm(&self, pitch_mm: f64) -> f64 {
        self.depth_units as f64 * pitch_mm
    }

    pub fn height_mm(&self) -> f64 {
        self.height_units as f64 * GRIDFINITY_HEIGHT_INCREMENT_MM
    }
}

/// Parameters for creating a new bin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewGridfinityBin {
    pub bin_code: String,
    pub width_units: u32, // suggestion: consider turning into vector
    pub depth_units: u32,
    pub height_units: u32,
    pub compartment_count: u32,
    pub parent_location_id: Option<LocationId>,
}

impl NewGridfinityBin {
    pub fn validate(&self) -> CoreResult<()> {
        if self.bin_code.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "bin code must not be empty".to_string(),
            ));
        }
        if self.width_units == 0 || self.depth_units == 0 || self.height_units == 0 {
            return Err(CoreError::InvalidInput(
                "bin dimensions must be greater than zero".to_string(),
            ));
        }
        if self.compartment_count == 0 {
            return Err(CoreError::InvalidInput(
                "compartment count must be at least 1".to_string(),
            ));
        }
        Ok(())
    }
}

/// A spatial placement of a bin on a baseplate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinPlacement {
    pub id: PlacementId,
    pub bin_id: GridfinityBinId,
    pub base_id: GridBaseId,
    pub x: u32,
    pub y: u32,
    pub rotation: Rotation,
    pub placed_at: OffsetDateTime,
    pub removed_at: Option<OffsetDateTime>,
}

impl BinPlacement {
    pub fn is_active(&self) -> bool {
        self.removed_at.is_none()
    }
}

/// Command to place a bin on a baseplate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceBinCmd {
    pub bin_id: GridfinityBinId,
    pub base_id: GridBaseId,
    pub x: u32,
    pub y: u32,
    pub rotation: Rotation,
}

/// Active placement information including bin metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivePlacementInfo {
    pub placement: BinPlacement,
    pub bin_code: String,
    pub width_units: u32,
    pub depth_units: u32,
    pub height_units: u32,
}

impl ActivePlacementInfo {
    /// Calculate effective dimensions of the placed bin.
    pub fn effective_dimensions(&self) -> (u32, u32) {
        match self.placement.rotation {
            Rotation::Rot0 => (self.width_units, self.depth_units),
            Rotation::Rot90 => (self.depth_units, self.width_units),
        }
    }

    /// Check if this placed bin covers coordinate (x, y).
    pub fn covers(&self, x: u32, y: u32) -> bool {
        let (ew, ed) = self.effective_dimensions();
        x >= self.placement.x
            && x < self.placement.x + ew
            && y >= self.placement.y
            && y < self.placement.y + ed
    }
}

/// Representation of a single unit cell in a 2D baseplate grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridCell {
    Empty,
    Occupied {
        bin_id: GridfinityBinId,
        bin_code: String,
        is_origin: bool,
    },
}

/// A 2D spatial map of a baseplate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridMap {
    pub base: GridBase,
    pub cells: Vec<Vec<GridCell>>, // [y][x]
    pub placed_bins: Vec<ActivePlacementInfo>,
}

impl GridMap {
    pub fn new(base: GridBase, placements: Vec<ActivePlacementInfo>) -> Self {
        let cols = base.cols as usize;
        let rows = base.rows as usize;
        let mut cells = vec![vec![GridCell::Empty; cols]; rows];

        for p in &placements {
            let (ew, ed) = p.effective_dimensions();
            for dy in 0..ed {
                for dx in 0..ew {
                    let cx = (p.placement.x + dx) as usize;
                    let cy = (p.placement.y + dy) as usize;
                    if cy < rows && cx < cols {
                        cells[cy][cx] = GridCell::Occupied {
                            bin_id: p.placement.bin_id,
                            bin_code: p.bin_code.clone(),
                            is_origin: dx == 0 && dy == 0,
                        };
                    }
                }
            }
        }

        Self {
            base,
            cells,
            placed_bins: placements,
        }
    }

    // Suggestion: consider turning into Display impl. (then you get .to_string())
    /// Format an ASCII representation of the 2D grid.
    pub fn render_ascii(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Baseplate '{}' ({}x{} cells, {:.1}mm pitch):\n",
            self.base.code, self.base.cols, self.base.rows, self.base.pitch_mm
        ));

        // Column header
        out.push_str("    ");
        for x in 0..self.base.cols {
            out.push_str(&format!("  X{:<2}", x));
        }
        out.push('\n');

        for (y, row) in self.cells.iter().enumerate() {
            out.push_str(&format!("Y{:<2} ", y));
            for cell in row {
                match cell {
                    GridCell::Empty => out.push_str(" [ . ]"),
                    GridCell::Occupied {
                        bin_code,
                        is_origin,
                        ..
                    } => {
                        let tag = if *is_origin {
                            format!("{:^3}", bin_code.chars().take(3).collect::<String>())
                        } else {
                            " # ".to_string()
                        };
                        out.push_str(&format!(" [{tag}]"));
                    }
                }
            }
            out.push('\n');
        }
        out
    }
}

/// Geometry helper functions for spatial placement checks.
pub mod geometry {
    use super::*;

    /// Validate that a bin placement fits within baseplate boundaries and does not collide
    /// with any existing active placements.
    pub fn validate_placement(
        base: &GridBase,
        bin: &GridfinityBin,
        cmd: &PlaceBinCmd,
        existing_placements: &[ActivePlacementInfo],
    ) -> CoreResult<()> {
        let (eff_w, eff_d) = bin.effective_dimensions(cmd.rotation);

        // 1. Bounds check
        if cmd.x.checked_add(eff_w).is_none_or(|end| end > base.cols) {
            return Err(CoreError::InvalidInput(format!(
                "bin '{bin_code}' (width {eff_w}) exceeds baseplate width (cols {cols}) at X={x}",
                bin_code = bin.bin_code,
                cols = base.cols,
                x = cmd.x
            )));
        }
        if cmd.y.checked_add(eff_d).is_none_or(|end| end > base.rows) {
            return Err(CoreError::InvalidInput(format!(
                "bin '{bin_code}' (depth {eff_d}) exceeds baseplate depth (rows {rows}) at Y={y}",
                bin_code = bin.bin_code,
                rows = base.rows,
                y = cmd.y
            )));
        }

        // 2. Collision check against active placements
        for existing in existing_placements {
            // Skip self if re-placing the same bin
            if existing.placement.bin_id == bin.id {
                continue;
            }

            let (ex_w, ex_d) = existing.effective_dimensions();
            let ex_x = existing.placement.x;
            let ex_y = existing.placement.y;

            // Check AABB overlap between [cmd.x, cmd.x + eff_w) and [ex_x, ex_x + ex_w)
            let x_overlap = cmd.x < ex_x + ex_w && cmd.x + eff_w > ex_x;
            let y_overlap = cmd.y < ex_y + ex_d && cmd.y + eff_d > ex_y;

            if x_overlap && y_overlap {
                return Err(CoreError::InvalidInput(format!(
                    "spatial collision: bin '{new_bin}' footprint ({eff_w}x{eff_d} at {x},{y}) overlaps existing bin '{ex_bin}' at ({ex_x},{ex_y})",
                    new_bin = bin.bin_code,
                    x = cmd.x,
                    y = cmd.y,
                    ex_bin = existing.bin_code,
                )));
            }
        }

        Ok(())
    }
}
