use std::collections::BTreeMap;
use std::io::Cursor;

use bson::{Bson, Document};

use crate::constants::tutorial;
use crate::models::{TileCount, WorldSnapshot};
use crate::protocol;

#[derive(Debug, Clone)]
pub struct DecodedWorld {
    pub snapshot: WorldSnapshot,
    pub foreground_tiles: Vec<u16>,
    pub background_tiles: Vec<u16>,
    pub water_tiles: Vec<u16>,
    pub wiring_tiles: Vec<u16>,
}

pub fn decode_gwc(world_name: Option<String>, raw: &[u8]) -> Result<DecodedWorld, String> {
    let decompressed =
        zstd::stream::decode_all(Cursor::new(raw)).map_err(|error| error.to_string())?;
    let document = Document::from_reader(Cursor::new(decompressed)).map_err(|error| error.to_string())?;
    parse_world_document(world_name, &document)
}

pub fn parse_world_document(
    world_name: Option<String>,
    world_bson: &Document,
) -> Result<DecodedWorld, String> {
    let size = world_bson
        .get_document("WorldSizeSettingsType")
        .cloned()
        .unwrap_or_default();
    let width = size.get_i32("WorldSizeX").unwrap_or_default().max(0) as u32;
    let height = size.get_i32("WorldSizeY").unwrap_or_default().max(0) as u32;

    // For TUTORIAL2 the GWC always ships WorldStartPoint = (40, 30) — that is the
    // generic observer spawn.  New accounts must spawn at the sleeping pod instead:
    // map (39, 44) = world (12.48, 13.92), confirmed from packets.bin rec 32.
    let (spawn_map_x, spawn_map_y) = if world_name.as_deref() == Some(tutorial::TUTORIAL_WORLD) {
        (
            Some(tutorial::TUTORIAL_SPAWN_MAP_X as f64),
            Some(tutorial::TUTORIAL_SPAWN_MAP_Y as f64),
        )
    } else {
        let spawn = world_bson
            .get_document("WorldStartPoint")
            .cloned()
            .unwrap_or_default();
        (spawn_value(&spawn, "x"), spawn_value(&spawn, "y"))
    };
    let (spawn_world_x, spawn_world_y) = match (spawn_map_x, spawn_map_y) {
        (Some(x), Some(y)) => {
            let (wx, wy) = protocol::map_to_world(x, y);
            (Some(wx), Some(wy))
        }
        _ => (None, None),
    };

    let block_layer = protocol::binary_bytes(world_bson.get("BlockLayer")).unwrap_or_default();
    let background_layer =
        protocol::binary_bytes(world_bson.get("BackgroundLayer")).unwrap_or_default();
    let water_layer = protocol::binary_bytes(world_bson.get("WaterLayer")).unwrap_or_default();
    let wiring_layer = protocol::binary_bytes(world_bson.get("WiringLayer")).unwrap_or_default();
    let mut tile_map = BTreeMap::<u16, u32>::new();
    let mut foreground_tiles = Vec::with_capacity(block_layer.len() / 2);
    for chunk in block_layer.chunks_exact(2) {
        let tile_id = u16::from_le_bytes([chunk[0], chunk[1]]);
        foreground_tiles.push(tile_id);
        *tile_map.entry(tile_id).or_insert(0) += 1;
    }
    let tile_counts = tile_map
        .into_iter()
        .map(|(tile_id, count)| TileCount { tile_id, count })
        .collect::<Vec<_>>();

    let collectables_count = document_len(world_bson.get("Collectables"));
    let world_items_count = document_len(world_bson.get("WorldItems"));

    Ok(DecodedWorld {
        snapshot: WorldSnapshot {
            world_name,
            width,
            height,
            spawn_map_x,
            spawn_map_y,
            spawn_world_x,
            spawn_world_y,
            collectables_count,
            world_items_count,
            tile_counts,
        },
        foreground_tiles,
        background_tiles: decode_layer(&background_layer),
        water_tiles: decode_layer(&water_layer),
        wiring_tiles: decode_layer(&wiring_layer),
    })
}

fn decode_layer(bytes: &[u8]) -> Vec<u16> {
    let mut tiles = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        tiles.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    tiles
}

fn spawn_value(document: &Document, key: &str) -> Option<f64> {
    match document.get(key) {
        Some(Bson::Double(value)) => Some(*value),
        Some(Bson::Int32(value)) => Some(*value as f64),
        Some(Bson::Int64(value)) => Some(*value as f64),
        _ => None,
    }
}

fn document_len(value: Option<&Bson>) -> usize {
    match value {
        Some(Bson::Document(document)) => document
            .iter()
            .filter(|(key, value)| key.as_str() != "Count" && matches!(value, Bson::Document(_)))
            .count(),
        _ => 0,
    }
}
