use anyhow::{Context, Result};
use qrcode::QrCode;
use qrcode::render::svg;
use serde::{Deserialize, Serialize};

const APP_IDENTIFIER: &str = "umbrel-balancebridge";
const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct PairingPayload {
    pub version: u32,
    pub app: String,
    #[serde(rename = "nodePubkey")]
    pub node_pubkey: String,
    pub relays: Vec<String>,
}

impl PairingPayload {
    pub fn new(node_pubkey: String, relays: Vec<String>) -> Self {
        Self {
            version: VERSION,
            app: APP_IDENTIFIER.to_string(),
            node_pubkey,
            relays,
        }
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .context("Failed to serialize pairing payload")
    }

    /// Generate QR code as SVG (stable, no image crate)
    pub fn generate_qr_svg(&self) -> Result<String> {
        let json = self.to_json()?;

        let code = QrCode::new(json.as_bytes())
            .context("Failed to generate QR code")?;

        let svg = code
            .render::<svg::Color>()
            .min_dimensions(512, 512)
            .dark_color(svg::Color("#000000"))
            .light_color(svg::Color("#ffffff"))
            .build();

        Ok(svg)
    }
}
