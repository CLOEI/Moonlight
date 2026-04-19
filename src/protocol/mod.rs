use std::io::Cursor;
use std::time::{SystemTime, UNIX_EPOCH};

use bson::{Binary, Bson, Document, doc, spec::BinarySubtype};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::constants::{movement, network, protocol as ids};

pub fn csharp_ticks() -> i64 {
    let unix_ticks = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs_f64())
        .unwrap_or_default();
    (unix_ticks * 10_000_000.0) as i64 + 621_355_968_000_000_000
}

pub fn wrap_batch(messages: &[Document]) -> Document {
    let mut outer = Document::new();
    for (index, message) in messages.iter().enumerate() {
        outer.insert(format!("m{index}"), Bson::Document(message.clone()));
    }
    outer.insert("mc", messages.len() as i32);
    outer
}

pub fn extract_messages(outer: &Document) -> Vec<Document> {
    let count = outer.get_i32("mc").unwrap_or_default().max(0) as usize;
    let mut messages = Vec::with_capacity(count);
    for index in 0..count {
        if let Some(Bson::Document(message)) = outer.get(&format!("m{index}")) {
            messages.push(message.clone());
        }
    }
    if messages.is_empty() && outer.contains_key("ID") {
        messages.push(outer.clone());
    }
    messages
}

pub async fn write_batch<W>(writer: &mut W, messages: &[Document]) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
{
    let packet = encode_batch(messages)?;
    writer
        .write_all(&packet)
        .await
        .map_err(|error| error.to_string())?;
    writer.flush().await.map_err(|error| error.to_string())
}

pub fn encode_batch(messages: &[Document]) -> Result<Vec<u8>, String> {
    let outer = wrap_batch(messages);
    let bson = outer.to_vec().map_err(|error| error.to_string())?;
    let total_len = (bson.len() + 4) as u32;
    let mut packet = Vec::with_capacity(bson.len() + 4);
    packet.extend_from_slice(&total_len.to_le_bytes());
    packet.extend_from_slice(&bson);
    Ok(packet)
}

pub async fn read_packet<R>(reader: &mut R) -> Result<Document, String>
where
    R: AsyncRead + Unpin,
{
    let total_len = reader
        .read_u32_le()
        .await
        .map_err(|error| error.to_string())?;
    if total_len < 4 {
        return Err(format!("invalid packet length {total_len}"));
    }

    let mut payload = vec![0u8; total_len as usize - 4];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|error| error.to_string())?;

    Document::from_reader(Cursor::new(payload)).map_err(|error| error.to_string())
}

pub fn binary_bytes(value: Option<&Bson>) -> Option<Vec<u8>> {
    match value {
        Some(Bson::Binary(binary)) => Some(binary.bytes.clone()),
        _ => None,
    }
}

pub fn summarize_messages(messages: &[Document]) -> String {
    if messages.is_empty() {
        return "empty".to_string();
    }

    messages
        .iter()
        .map(summarize_message)
        .collect::<Vec<_>>()
        .join(" | ")
}

pub fn summarize_message(message: &Document) -> String {
    let id = message.get_str("ID").unwrap_or("?");
    let summary = match id {
        "TTjW" => format!("TTjW W={}", message.get_str("W").unwrap_or("?")),
        "Gw" => format!("Gw W={}", message.get_str("W").unwrap_or("?")),
        "ULS" => format!("ULS LS={}", message.get_str("LS").unwrap_or("?")),
        "GWC" => {
            let bytes = binary_bytes(message.get("W")).unwrap_or_default();
            format!("GWC compressed={}B", bytes.len())
        }
        "OoIP" => format!(
            "OoIP IP={} WN={}",
            message.get_str("IP").unwrap_or("?"),
            message.get_str("WN").unwrap_or("?")
        ),
        "GPd" => format!(
            "GPd U={} UN={}",
            message.get_str("U").unwrap_or("?"),
            message.get_str("UN").unwrap_or("?")
        ),
        _ => id.to_string(),
    };

    format!("{summary} {}", compact_document(message))
}

fn compact_document(document: &Document) -> String {
    let items = document
        .iter()
        .map(|(key, value)| format!("{key}={}", compact_bson(value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{{{items}}}")
}

fn compact_bson(value: &Bson) -> String {
    match value {
        Bson::String(value) => format!("{value:?}"),
        Bson::Int32(value) => value.to_string(),
        Bson::Int64(value) => value.to_string(),
        Bson::Double(value) => format!("{value}"),
        Bson::Boolean(value) => value.to_string(),
        Bson::Null => "null".to_string(),
        Bson::Binary(binary) => format!("<binary:{}B>", binary.bytes.len()),
        Bson::Document(document) => compact_document(document),
        Bson::Array(items) => {
            let rendered = items
                .iter()
                .map(compact_bson)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{rendered}]")
        }
        other => format!("{other:?}"),
    }
}

pub fn make_vchk(device_id: &str) -> Document {
    doc! {
        "ID": ids::PACKET_ID_VCHK,
        "OS": "WindowsPlayer",
        "OSt": 3,
        "sdid": device_id,
    }
}

pub fn make_gpd(jwt: &str) -> Document {
    doc! {
        "ID": ids::PACKET_ID_GPD,
        "AT": jwt,
        "cgy": 877,
        "Pw": network::RELAUNCH_PASS,
    }
}

pub fn make_st() -> Document {
    doc! {
        "ID": ids::PACKET_ID_ST,
        "T": csharp_ticks(),
    }
}

pub fn make_keepalive() -> Document {
    doc! { "ID": ids::PACKET_ID_KEEPALIVE }
}

pub fn make_empty_movement() -> Document {
    doc! { "ID": ids::PACKET_ID_MOVEMENT }
}

pub fn make_menu_transition() -> Vec<Document> {
    vec![
        make_wreu(),
        make_bcsu(),
        make_update_location("#menu"),
        doc! { "ID": ids::PACKET_ID_DAILY_BONUS },
        make_st(),
    ]
}

pub fn make_glsi() -> Vec<Document> {
    vec![doc! { "ID": ids::PACKET_ID_GET_LSI }, make_st()]
}

pub fn make_join_world(world: &str) -> Document {
    doc! {
        "ID": ids::PACKET_ID_JOIN_WORLD,
        "W": world.to_uppercase(),
        "WB": 0,
        "Amt": 0,
    }
}

pub fn make_join_world_special(world: &str, biome: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_JOIN_WORLD,
        "Is": true,
        "W": world.to_uppercase(),
        "WB": biome,
        "Amt": 0,
    }
}

pub fn make_world_load_args(args: &[i32]) -> Document {
    doc! {
        "ID": ids::PACKET_ID_WORLD_LOAD_ARGS,
        "WCSD": args.iter().copied().collect::<Vec<_>>(),
    }
}

pub fn make_enter_world(world: &str) -> Vec<Document> {
    make_enter_world_eid(world, "")
}

pub fn make_enter_world_eid(world: &str, eid: &str) -> Vec<Document> {
    vec![
        doc! { "ID": ids::PACKET_ID_GET_WORLD, "eID": eid, "W": world, "WB": 0 },
        doc! { "ID": "A", "AE": 2 },
        doc! { "ID": "A", "AE": 6 },
        doc! { "ID": "A", "AE": 14 },
        doc! { "ID": "A", "AE": 23 },
        doc! { "ID": "GSb" },
    ]
}

pub fn make_spawn_location_sync(world: &str) -> Vec<Document> {
    vec![make_update_location(world), make_st()]
}

pub fn make_spawn_setup() -> Vec<Document> {
    vec![
        doc! { "ID": "cZL", "CZL": 2 },
        doc! { "ID": "cZva", "Amt": 0.5 },
        doc! { "ID": ids::PACKET_ID_R_OP },
        doc! { "ID": "rAIp" },
        doc! { "ID": ids::PACKET_ID_R_AI },
    ]
}

pub fn make_ready_to_play() -> Vec<Document> {
    vec![doc! { "ID": ids::PACKET_ID_READY_TO_PLAY }, make_st()]
}

pub fn make_leave_world() -> Document {
    doc! { "ID": ids::PACKET_ID_LEAVE_WORLD }
}

pub fn make_character_create(gender: i32, country: i32, skin_color: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_CHARACTER_CREATE,
        "Gnd": gender,
        "Ctry": country,
        "SCI": skin_color,
    }
}

pub fn make_wear_item(block_id: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_WEAR_ITEM,
        "hBlock": block_id,
    }
}

pub fn make_unwear_item(block_id: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_UNWEAR_ITEM,
        "hBlock": block_id,
    }
}

pub fn make_select_belt_item(inventory_key: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_SELECT_BELT_ITEM,
        "Bi": inventory_key,
    }
}

pub fn make_place_block(target_x: i32, target_y: i32, block_id: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_SET_BLOCK,
        "x": target_x,
        "y": target_y,
        "BlockType": block_id,
    }
}

pub fn make_hit_block(target_x: i32, target_y: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_HIT_BLOCK,
        "x": target_x,
        "y": target_y,
    }
}

pub fn make_hit_block_water(map_x: i32, map_y: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_HIT_BLOCK_WATER,
        "x": map_x,
        "y": map_y,
    }
}

pub fn make_hit_block_background(target_x: i32, target_y: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_HIT_BLOCK_BG,
        "x": target_x,
        "y": target_y,
    }
}

pub fn make_seed_block(target_x: i32, target_y: i32, block_id: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_SEED_BLOCK,
        "x": target_x,
        "y": target_y,
        "BlockType": block_id,
    }
}

pub fn make_collectable_request(collectable_id: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_COLLECTABLE_REQUEST,
        "CollectableID": collectable_id,
    }
}

pub fn make_progress_signal(value: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_PROGRESS_SIGNAL,
        "SIc": value,
    }
}

pub fn make_buy_item_pack(pack_id: &str) -> Document {
    doc! {
        "ID": ids::PACKET_ID_BUY_ITEM_PACK,
        "IPId": pack_id,
    }
}

pub fn make_action_event(action_event: i32) -> Document {
    doc! {
        "ID": "A",
        "AE": action_event,
    }
}

pub fn make_action_apu(values: &[i32]) -> Document {
    doc! {
        "ID": "A",
        "APu": values.iter().copied().collect::<Vec<_>>(),
    }
}

pub fn make_ui_event_count(value: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_UI_EVENT_COUNT,
        "iEsC": value,
    }
}

pub fn make_ui_gift_view(vt: i32, vv: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_UI_GIFT_VIEW,
        "VT": vt,
        "VV": vv,
    }
}

pub fn make_floating_chest_refresh() -> Document {
    doc! { "ID": ids::PACKET_ID_UI_FLOATING_CHEST }
}

pub fn make_world_gift_request() -> Document {
    doc! { "ID": ids::PACKET_ID_WORLD_GIFT_REQUEST }
}

pub fn make_floating_gift_poll() -> Document {
    doc! { "ID": "FtGP", "FtWi": true }
}

pub fn make_bsw() -> Document {
    doc! { "ID": "BSW" }
}

pub fn make_tstate(value: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_TSTATE,
        "Tstate": value,
    }
}

pub fn make_activate_out_portal(map_x: i32, map_y: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_PORTAL_OUT,
        "x": map_x,
        "y": map_y,
    }
}

pub fn make_portal_arrive(map_x: i32, map_y: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_PORTAL_IN,
        "x": map_x,
        "y": map_y,
    }
}

pub fn make_wreu() -> Document {
    doc! {
        "ID": ids::PACKET_ID_WREU,
        "WREgA": true,
    }
}

pub fn make_bcsu() -> Document {
    doc! { "ID": ids::PACKET_ID_BCSU }
}

pub fn make_update_location(location: &str) -> Document {
    doc! {
        "ID": ids::PACKET_ID_UPDATE_LOCATION,
        "LS": location,
    }
}

pub fn make_map_point(map_x: i32, map_y: i32) -> Document {
    let mut point = Vec::with_capacity(8);
    point.extend_from_slice(&map_x.to_le_bytes());
    point.extend_from_slice(&map_y.to_le_bytes());

    doc! {
        "ID": ids::PACKET_ID_MAP_POINT,
        "pM": Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: point,
        })
    }
}

pub fn make_movement_packet(
    world_x: f64,
    world_y: f64,
    anim: i32,
    direction: i32,
    teleport: bool,
) -> Document {
    let mut doc = doc! {
        "ID": ids::PACKET_ID_MOVEMENT,
        "x": world_x,
        "y": world_y,
        "t": csharp_ticks(),
        "a": anim,
        "d": direction,
    };
    if teleport {
        doc.insert("tp", true);
    }
    doc
}

pub fn make_move_to_map_point(map_x: i32, map_y: i32, anim: i32, direction: i32) -> Vec<Document> {
    let (world_x, world_y) = map_to_world(map_x as f64, map_y as f64);
    vec![
        make_map_point(map_x, map_y),
        make_movement_packet(world_x, world_y, anim, direction, false),
    ]
}

pub fn make_spawn_packets(map_x: i32, map_y: i32, world_x: f64, world_y: f64) -> Vec<Document> {
    vec![
        make_map_point(map_x, map_y),
        make_movement_packet(
            world_x,
            world_y,
            movement::ANIM_IDLE,
            movement::DIR_RIGHT,
            true,
        ),
    ]
}

pub fn make_try_to_fish_from_map_point(
    target_x: i32,
    target_y: i32,
    bait_block_id: i32,
) -> Document {
    doc! {
        "ID": ids::PACKET_ID_TRY_TO_FISH_FROM_MAP_POINT,
        "x": target_x,
        "y": target_y,
        "BT": bait_block_id,
    }
}

pub fn make_start_fishing_game(target_x: i32, target_y: i32, bait_block_id: i32) -> Document {
    doc! {
        "ID": ids::PACKET_ID_START_FISHING_GAME,
        "MGT": 2,
        "x": target_x,
        "y": target_y,
        "BT": bait_block_id,
    }
}

pub fn make_fishing_hook_action() -> Document {
    doc! {
        "ID": ids::PACKET_ID_FISHING_GAME_ACTION,
        "MGT": 2,
        "MGD": csharp_ticks(),
        "LS": 2,
    }
}

pub fn make_fishing_land_action(vendor_index: i32, index_key: i32, amount: f64) -> Document {
    doc! {
        "ID": ids::PACKET_ID_FISHING_GAME_ACTION,
        "MGT": 2,
        "MGD": csharp_ticks(),
        "LS": 1,
        "vI": vendor_index,
        "Idx": index_key,
        "Amt": amount,
    }
}

pub fn make_stop_fishing_game(finalize: bool) -> Document {
    let mut document = doc! {
        "ID": ids::PACKET_ID_STOP_MINIGAME,
        "MGT": 2,
    };
    if finalize {
        document.insert("MGA", 1);
    }
    document
}

pub fn make_fish_on_area() -> Document {
    doc! { "ID": ids::PACKET_ID_FISH_ON_AREA }
}

pub fn make_fish_off_area(distance: f64) -> Document {
    doc! {
        "ID": ids::PACKET_ID_FISH_OFF_AREA,
        "FiD": distance,
    }
}

pub fn make_world_chat(message: &str) -> Document {
    doc! {
        "ID": ids::PACKET_ID_WORLD_CHAT,
        "msg": message,
    }
}

pub fn make_fishing_cleanup_action() -> Document {
    doc! {
        "ID": ids::PACKET_ID_FISHING_GAME_ACTION,
        "MGT": 2,
        "MGD": 1,
        "LS": 0,
    }
}

pub fn map_to_world(map_x: f64, map_y: f64) -> (f64, f64) {
    (
        map_x * movement::TILE_WIDTH,
        map_y * movement::TILE_HEIGHT - (0.5 * movement::TILE_HEIGHT),
    )
}

pub fn world_to_map(world_x: f64, world_y: f64) -> (f64, f64) {
    (
        world_x / movement::TILE_WIDTH,
        (world_y + (0.5 * movement::TILE_HEIGHT)) / movement::TILE_HEIGHT,
    )
}

#[cfg(test)]
mod tests {
    use super::{encode_batch, extract_messages, make_vchk, wrap_batch};

    #[test]
    fn batch_round_trip_preserves_messages() {
        let messages = vec![make_vchk("abc")];
        let batch = wrap_batch(&messages);
        let extracted = extract_messages(&batch);
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].get_str("ID").unwrap(), "VChk");
    }

    #[test]
    fn batch_encoding_has_length_prefix() {
        let bytes = encode_batch(&[make_vchk("abc")]).unwrap();
        let len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
        assert_eq!(len, bytes.len());
    }
}
