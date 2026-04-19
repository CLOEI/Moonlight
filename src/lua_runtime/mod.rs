use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bson::{Bson, Document};
use mlua::{Error as LuaError, Lua, Table, Value};
use tokio::runtime::Handle;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::logging::Logger;
use crate::models::{
    LuaCollectableSnapshot, LuaGrowingTileSnapshot, LuaScriptStatusSnapshot, LuaTileSnapshot,
    LuaWorldSnapshot,
};
use crate::session::BotSession;

#[derive(Debug)]
pub struct LuaScriptHandle {
    pub cancel: Arc<AtomicBool>,
    pub status: Arc<RwLock<LuaScriptStatusSnapshot>>,
    pub task: JoinHandle<()>,
}

pub fn idle_status() -> LuaScriptStatusSnapshot {
    LuaScriptStatusSnapshot {
        running: false,
        started_at: None,
        finished_at: None,
        last_error: None,
        last_result_message: None,
    }
}

pub fn spawn_script(session: Arc<BotSession>, source: String, logger: Logger) -> LuaScriptHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let status = Arc::new(RwLock::new(LuaScriptStatusSnapshot {
        running: true,
        started_at: Some(now_millis()),
        finished_at: None,
        last_error: None,
        last_result_message: None,
    }));
    let runtime = Handle::current();
    let session_id = session.id_string();
    let cancel_clone = cancel.clone();
    let status_clone = status.clone();
    let logger_clone = logger.clone();

    let task = tokio::task::spawn_blocking(move || {
        logger_clone.state(Some(&session_id), "lua script started");
        let result = run_script(session, source, cancel_clone.clone(), runtime);
        let mut next = match result {
            Ok(message) => LuaScriptStatusSnapshot {
                running: false,
                started_at: status_clone.blocking_read().started_at,
                finished_at: Some(now_millis()),
                last_error: None,
                last_result_message: Some(message),
            },
            Err(error) => LuaScriptStatusSnapshot {
                running: false,
                started_at: status_clone.blocking_read().started_at,
                finished_at: Some(now_millis()),
                last_error: Some(error.clone()),
                last_result_message: None,
            },
        };

        if cancel_clone.load(Ordering::Relaxed)
            && next.last_error.as_deref() == Some("lua script stopped")
        {
            next.last_error = None;
            next.last_result_message = Some("script stopped".to_string());
        }

        *status_clone.blocking_write() = next.clone();
        if let Some(error) = next.last_error {
            logger_clone.error(
                "lua",
                Some(&session_id),
                format!("lua script failed: {error}"),
            );
        } else if let Some(message) = next.last_result_message {
            logger_clone.state(Some(&session_id), format!("lua script finished: {message}"));
        }
    });

    LuaScriptHandle {
        cancel,
        status,
        task,
    }
}

fn run_script(
    session: Arc<BotSession>,
    source: String,
    cancel: Arc<AtomicBool>,
    runtime: Handle,
) -> Result<String, String> {
    let lua = Lua::new();
    let bot = build_bot_table(&lua, session, cancel, runtime).map_err(|error| error.to_string())?;
    lua.globals()
        .set("bot", bot)
        .map_err(|error| error.to_string())?;
    lua.load(&source)
        .set_name("session_script")
        .exec()
        .map_err(|error| error.to_string())?;
    Ok("script completed".to_string())
}

fn build_bot_table(
    lua: &Lua,
    session: Arc<BotSession>,
    cancel: Arc<AtomicBool>,
    runtime: Handle,
) -> mlua::Result<Table> {
    let bot = lua.create_table()?;

    bot.set(
        "warp",
        create_async_method(
            lua,
            session.clone(),
            cancel.clone(),
            runtime.clone(),
            |session, cancel, runtime, (_, world): (Value, String)| {
                runtime.block_on(session.warp(&world, &cancel))
            },
        )?,
    )?;
    bot.set(
        "warpInstance",
        create_async_method(
            lua,
            session.clone(),
            cancel.clone(),
            runtime.clone(),
            |session, cancel, runtime, (_, world): (Value, String)| {
                runtime.block_on(session.warp_instance(&world, &cancel))
            },
        )?,
    )?;
    bot.set(
        "walk",
        create_async_method(
            lua,
            session.clone(),
            cancel.clone(),
            runtime.clone(),
            |session, cancel, runtime, (_, dx, dy): (Value, i32, i32)| {
                runtime.block_on(session.walk(dx, dy, &cancel))
            },
        )?,
    )?;
    bot.set(
        "walkTo",
        create_async_method(
            lua,
            session.clone(),
            cancel.clone(),
            runtime.clone(),
            |session, cancel, runtime, (_, x, y): (Value, i32, i32)| {
                runtime.block_on(session.walk_to(x, y, &cancel))
            },
        )?,
    )?;
    bot.set("findPath", {
        let session = session.clone();
        let runtime = runtime.clone();
        lua.create_function(move |lua, (_self, x, y): (Value, i32, i32)| {
            let path = runtime
                .block_on(session.find_path(x, y))
                .map_err(LuaError::external)?;
            let out = lua.create_table()?;
            for (index, (px, py)) in path.into_iter().enumerate() {
                let point = lua.create_table()?;
                point.set("x", px)?;
                point.set("y", py)?;
                out.set(index + 1, point)?;
            }
            Ok(out)
        })?
    })?;

    let runtime_clone = runtime.clone();
    let session_clone = session.clone();
    let cancel_clone = cancel.clone();
    bot.set(
        "punch",
        create_async_method(
            lua,
            session_clone,
            cancel_clone,
            runtime_clone,
            |session, cancel, runtime, (_, dx, dy): (Value, i32, i32)| {
                runtime.block_on(session.punch(dx, dy, &cancel))
            },
        )?,
    )?;
    bot.set(
        "place",
        create_async_method(
            lua,
            session.clone(),
            cancel.clone(),
            runtime.clone(),
            |session, cancel, runtime, (_, dx, dy, block_id): (Value, i32, i32, i32)| {
                runtime.block_on(session.place(dx, dy, block_id, &cancel))
            },
        )?,
    )?;
    bot.set(
        "wear",
        create_async_method(
            lua,
            session.clone(),
            cancel.clone(),
            runtime.clone(),
            |session, cancel, runtime, (_, block_id): (Value, i32)| {
                runtime.block_on(session.wear(block_id, true, &cancel))
            },
        )?,
    )?;
    bot.set(
        "unwear",
        create_async_method(
            lua,
            session.clone(),
            cancel.clone(),
            runtime.clone(),
            |session, cancel, runtime, (_, block_id): (Value, i32)| {
                runtime.block_on(session.wear(block_id, false, &cancel))
            },
        )?,
    )?;
    bot.set(
        "talk",
        create_async_method(
            lua,
            session.clone(),
            cancel.clone(),
            runtime.clone(),
            |session, cancel, runtime, (_, message): (Value, String)| {
                runtime.block_on(session.talk(&message, &cancel))
            },
        )?,
    )?;
    bot.set(
        "collect",
        create_async_method(
            lua,
            session.clone(),
            cancel.clone(),
            runtime.clone(),
            |session, cancel, runtime, (_self,): (Value,)| {
                runtime.block_on(session.collect(&cancel))
            },
        )?,
    )?;
    bot.set("sleep", {
        let cancel = cancel.clone();
        let runtime = runtime.clone();
        lua.create_function(move |_lua, (_self, ms): (Value, i64)| {
            sleep_with_cancel(&runtime, &cancel, ms).map_err(LuaError::external)
        })?
    })?;
    bot.set("sendPacket", {
        let session = session.clone();
        let cancel = cancel.clone();
        let runtime = runtime.clone();
        lua.create_function(move |_lua, (_self, table): (Value, Table)| {
            let packet = lua_table_to_document(table).map_err(LuaError::external)?;
            runtime
                .block_on(session.send_packet(packet, &cancel))
                .map_err(LuaError::external)
        })?
    })?;
    bot.set("getPosition", {
        let session = session.clone();
        let runtime = runtime.clone();
        lua.create_function(move |lua, (_self,): (Value,)| {
            let position = runtime.block_on(session.position());
            let table = lua.create_table()?;
            set_option(&table, "map_x", position.map_x)?;
            set_option(&table, "map_y", position.map_y)?;
            set_option(&table, "world_x", position.world_x)?;
            set_option(&table, "world_y", position.world_y)?;
            Ok(table)
        })?
    })?;
    bot.set("getCurrentWorld", {
        let session = session.clone();
        let runtime = runtime.clone();
        lua.create_function(move |_lua, (_self,): (Value,)| {
            Ok(runtime.block_on(session.current_world()))
        })?
    })?;
    bot.set("getStatus", {
        let session = session.clone();
        let runtime = runtime.clone();
        lua.create_function(move |_lua, (_self,): (Value,)| {
            Ok(runtime.block_on(session.status()).as_str())
        })?
    })?;
    bot.set("isInWorld", {
        let session = session.clone();
        let runtime = runtime.clone();
        lua.create_function(move |_lua, (_self,): (Value,)| {
            Ok(runtime.block_on(session.is_in_world()))
        })?
    })?;
    bot.set("getInventoryCount", {
        let session = session.clone();
        let runtime = runtime.clone();
        lua.create_function(move |_lua, (_self, block_id): (Value, u16)| {
            Ok(runtime.block_on(session.inventory_count(block_id)))
        })?
    })?;
    bot.set("getCollectables", {
        let session = session.clone();
        let runtime = runtime.clone();
        lua.create_function(move |lua, (_self,): (Value,)| {
            let collectables = runtime.block_on(session.collectables());
            collectables_table(lua, &collectables)
        })?
    })?;
    bot.set("isTileReadyToHarvest", {
        let session = session.clone();
        let runtime = runtime.clone();
        lua.create_function(move |_lua, (_self, x, y): (Value, i32, i32)| {
            runtime
                .block_on(session.is_tile_ready_to_harvest(x, y))
                .map_err(LuaError::external)
        })?
    })?;
    bot.set("getWorld", {
        let session = session.clone();
        let runtime = runtime.clone();
        lua.create_function(move |lua, (_self,): (Value,)| {
            let world = runtime
                .block_on(session.world())
                .map_err(LuaError::external)?;
            build_world_table(lua, world)
        })?
    })?;

    Ok(bot)
}

fn create_async_method<A, F>(
    lua: &Lua,
    session: Arc<BotSession>,
    cancel: Arc<AtomicBool>,
    runtime: Handle,
    callback: F,
) -> mlua::Result<mlua::Function>
where
    A: mlua::FromLuaMulti,
    F: Fn(Arc<BotSession>, Arc<AtomicBool>, Handle, A) -> Result<(), String> + Send + 'static,
{
    lua.create_function(move |_lua, args: A| {
        callback(session.clone(), cancel.clone(), runtime.clone(), args).map_err(LuaError::external)
    })
}

fn build_world_table(lua: &Lua, world: LuaWorldSnapshot) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("name", world.name.clone())?;
    table.set("width", world.width)?;
    table.set("height", world.height)?;

    let spawn = lua.create_table()?;
    set_option(&spawn, "map_x", world.spawn.map_x)?;
    set_option(&spawn, "map_y", world.spawn.map_y)?;
    set_option(&spawn, "world_x", world.spawn.world_x)?;
    set_option(&spawn, "world_y", world.spawn.world_y)?;
    table.set("spawn", spawn)?;

    let tiles = lua.create_table()?;
    tiles.set(
        "foreground",
        lua.create_sequence_from(world.tiles.foreground.clone())?,
    )?;
    tiles.set(
        "background",
        lua.create_sequence_from(world.tiles.background.clone())?,
    )?;
    tiles.set(
        "water",
        lua.create_sequence_from(world.tiles.water.clone())?,
    )?;
    tiles.set(
        "wiring",
        lua.create_sequence_from(world.tiles.wiring.clone())?,
    )?;
    table.set("tiles", tiles)?;

    let objects = lua.create_table()?;
    objects.set(
        "collectables",
        collectables_table(lua, &world.objects.collectables)?,
    )?;
    objects.set(
        "growing_tiles",
        growing_tiles_table(lua, &world.objects.growing_tiles)?,
    )?;
    table.set("objects", objects)?;

    let world_clone = world.clone();
    table.set(
        "getTile",
        lua.create_function(move |lua, (_self, x, y): (Value, i32, i32)| {
            world_tile_table(lua, &world_clone, x, y)
        })?,
    )?;
    let world_clone = world.clone();
    table.set(
        "isTileReadyToHarvest",
        lua.create_function(move |_lua, (_self, x, y): (Value, i32, i32)| {
            world_tile_ready(&world_clone, x, y).map_err(LuaError::external)
        })?,
    )?;

    Ok(table)
}

fn world_tile_table(lua: &Lua, world: &LuaWorldSnapshot, x: i32, y: i32) -> mlua::Result<Table> {
    let tile = world_tile(world, x, y).map_err(LuaError::external)?;
    let table = lua.create_table()?;
    table.set("foreground", tile.foreground)?;
    table.set("background", tile.background)?;
    table.set("water", tile.water)?;
    table.set("wiring", tile.wiring)?;
    table.set("ready_to_harvest", tile.ready_to_harvest)?;
    Ok(table)
}

fn world_tile(world: &LuaWorldSnapshot, x: i32, y: i32) -> Result<LuaTileSnapshot, String> {
    if x < 0 || y < 0 || x >= world.width as i32 || y >= world.height as i32 {
        return Err(format!("tile ({x}, {y}) is out of bounds"));
    }
    let index = y as usize * world.width as usize + x as usize;
    Ok(LuaTileSnapshot {
        foreground: world
            .tiles
            .foreground
            .get(index)
            .copied()
            .unwrap_or_default(),
        background: world
            .tiles
            .background
            .get(index)
            .copied()
            .unwrap_or_default(),
        water: world.tiles.water.get(index).copied().unwrap_or_default(),
        wiring: world.tiles.wiring.get(index).copied().unwrap_or_default(),
        ready_to_harvest: world_tile_ready(world, x, y)?,
    })
}

fn world_tile_ready(world: &LuaWorldSnapshot, x: i32, y: i32) -> Result<bool, String> {
    if x < 0 || y < 0 || x >= world.width as i32 || y >= world.height as i32 {
        return Err(format!("tile ({x}, {y}) is out of bounds"));
    }
    let now_ticks = crate::protocol::csharp_ticks();
    Ok(world
        .objects
        .growing_tiles
        .iter()
        .find(|item| item.x == x && item.y == y)
        .map(|item| now_ticks >= item.growth_end_time)
        .unwrap_or(false))
}

fn collectables_table(lua: &Lua, collectables: &[LuaCollectableSnapshot]) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for (index, item) in collectables.iter().enumerate() {
        let table = lua.create_table()?;
        table.set("id", item.id)?;
        table.set("block_type", item.block_type)?;
        table.set("amount", item.amount)?;
        table.set("inventory_type", item.inventory_type)?;
        table.set("pos_x", item.pos_x)?;
        table.set("pos_y", item.pos_y)?;
        table.set("is_gem", item.is_gem)?;
        out.set(index + 1, table)?;
    }
    Ok(out)
}

fn growing_tiles_table(lua: &Lua, items: &[LuaGrowingTileSnapshot]) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for (index, item) in items.iter().enumerate() {
        let table = lua.create_table()?;
        table.set("x", item.x)?;
        table.set("y", item.y)?;
        table.set("block_id", item.block_id)?;
        table.set("growth_end_time", item.growth_end_time)?;
        table.set("growth_duration_secs", item.growth_duration_secs)?;
        table.set("mixed", item.mixed)?;
        table.set("harvest_seeds", item.harvest_seeds)?;
        table.set("harvest_blocks", item.harvest_blocks)?;
        table.set("harvest_gems", item.harvest_gems)?;
        table.set("harvest_extra_blocks", item.harvest_extra_blocks)?;
        out.set(index + 1, table)?;
    }
    Ok(out)
}

fn set_option<T>(table: &Table, key: &str, value: Option<T>) -> mlua::Result<()>
where
    T: mlua::IntoLua,
{
    match value {
        Some(value) => table.set(key, value),
        None => table.set(key, Value::Nil),
    }
}

fn sleep_with_cancel(runtime: &Handle, cancel: &AtomicBool, ms: i64) -> Result<(), String> {
    if ms < 0 {
        return Err("sleep duration must be non-negative".to_string());
    }
    let total = Duration::from_millis(ms as u64);
    runtime.block_on(async {
        let start = tokio::time::Instant::now();
        while start.elapsed() < total {
            if cancel.load(Ordering::Relaxed) {
                return Err("lua script stopped".to_string());
            }
            let remaining = total.saturating_sub(start.elapsed());
            tokio::time::sleep(remaining.min(Duration::from_millis(50))).await;
        }
        Ok(())
    })
}

fn lua_table_to_document(table: Table) -> Result<Document, String> {
    match lua_table_to_bson(table)? {
        Bson::Document(document) => Ok(document),
        _ => Err("packet must be a Lua table with string keys".to_string()),
    }
}

fn lua_table_to_bson(table: Table) -> Result<Bson, String> {
    if table.raw_len() > 0 {
        let mut items = Vec::new();
        for index in 1..=table.raw_len() {
            let value = table.raw_get(index).map_err(|error| error.to_string())?;
            items.push(lua_value_to_bson(value)?);
        }
        Ok(Bson::Array(items))
    } else {
        let mut document = Document::new();
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = pair.map_err(|error| error.to_string())?;
            let key = match key {
                Value::String(key) => key.to_str().map_err(|error| error.to_string())?.to_string(),
                _ => return Err("document keys must be strings".to_string()),
            };
            document.insert(key, lua_value_to_bson(value)?);
        }
        Ok(Bson::Document(document))
    }
}

fn lua_value_to_bson(value: Value) -> Result<Bson, String> {
    match value {
        Value::Nil => Ok(Bson::Null),
        Value::Boolean(value) => Ok(Bson::Boolean(value)),
        Value::Integer(value) => Ok(Bson::Int64(value)),
        Value::Number(value) => Ok(Bson::Double(value)),
        Value::String(value) => Ok(Bson::String(
            value
                .to_str()
                .map_err(|error| error.to_string())?
                .to_string(),
        )),
        Value::Table(table) => lua_table_to_bson(table),
        other => Err(format!(
            "unsupported Lua value in packet: {}",
            other.type_name()
        )),
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{idle_status, lua_table_to_document, sleep_with_cancel, world_tile_ready};
    use crate::models::{
        LuaWorldObjectsSnapshot, LuaWorldSnapshot, LuaWorldSpawnSnapshot, LuaWorldTilesSnapshot,
    };
    use mlua::Lua;
    use std::sync::atomic::AtomicBool;
    use tokio::runtime::Builder;

    #[test]
    fn raw_packet_conversion_requires_string_id() {
        let lua = Lua::new();
        let table = lua.create_table().unwrap();
        table.set("ID", "HB").unwrap();
        table.set("x", 1).unwrap();
        table.set("y", 2).unwrap();

        let document = lua_table_to_document(table).unwrap();
        assert_eq!(document.get_str("ID").unwrap(), "HB");
        assert_eq!(document.get_i64("x").unwrap(), 1);
        assert_eq!(document.get_i64("y").unwrap(), 2);
        assert_eq!(idle_status().running, false);
    }

    #[test]
    fn sleep_rejects_negative_duration() {
        let runtime = Builder::new_current_thread().enable_time().build().unwrap();
        let cancel = AtomicBool::new(false);
        let error = sleep_with_cancel(runtime.handle(), &cancel, -1).unwrap_err();
        assert!(error.contains("non-negative"));
    }

    #[test]
    fn world_ready_check_uses_growth_end_time() {
        let world = LuaWorldSnapshot {
            name: Some("TEST".to_string()),
            width: 2,
            height: 2,
            spawn: LuaWorldSpawnSnapshot {
                map_x: None,
                map_y: None,
                world_x: None,
                world_y: None,
            },
            tiles: LuaWorldTilesSnapshot {
                foreground: vec![0; 4],
                background: vec![0; 4],
                water: vec![0; 4],
                wiring: vec![0; 4],
            },
            objects: LuaWorldObjectsSnapshot {
                collectables: Vec::new(),
                growing_tiles: vec![crate::models::LuaGrowingTileSnapshot {
                    x: 1,
                    y: 1,
                    block_id: 2,
                    growth_end_time: i64::MIN,
                    growth_duration_secs: 31,
                    mixed: false,
                    harvest_seeds: 0,
                    harvest_blocks: 5,
                    harvest_gems: 0,
                    harvest_extra_blocks: 0,
                }],
            },
        };
        assert!(world_tile_ready(&world, 1, 1).unwrap());
    }
}
