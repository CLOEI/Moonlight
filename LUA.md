# Lua API

Moonlight includes a per-session Lua runtime powered by `mlua`.

- One active Lua script can run per session.
- Starting a new script for the same session stops the old one first.
- Scripts are submitted as source text over HTTP.
- Lua methods use `:` method-call style, for example `bot:talk("hello")`.

## HTTP API

Start a script:

```http
POST /api/sessions/{id}/lua/start
Content-Type: application/json
```

```json
{
  "source": "bot:talk('hello from lua')"
}
```

Stop a script:

```http
POST /api/sessions/{id}/lua/stop
```

Get script status:

```http
GET /api/sessions/{id}/lua/status
```

Status fields:

- `running`
- `started_at`
- `finished_at`
- `last_error`
- `last_result_message`

## Bot Methods

### Movement and actions

```lua
bot:warp(world)
bot:warpInstance(world)
bot:walk(dx, dy)
bot:walkTo(x, y)
bot:findPath(x, y)
bot:punch(dx, dy)
bot:place(dx, dy, block_id)
bot:wear(block_id)
bot:unwear(block_id)
bot:talk(message)
bot:collect()
bot:sleep(ms)
bot:sendPacket(packet)
```

Notes:

- `warp(world)` joins the target world and waits until the session is fully in that world.
- `warpInstance(world)` joins an instance/special world (e.g. `NETHERWORLD`). Sends `wlA` + `TTjW` with the `Is=true` flag. The server requires a consumable scroll in inventory.
- `walk(dx, dy)` uses relative tile offsets from the current player tile.
- `walkTo(x, y)` moves to an absolute map tile.
- `findPath(x, y)` returns a Lua array of points like `{ { x = 10, y = 20 }, ... }`.
- `punch(dx, dy)` and `place(dx, dy, block_id)` use relative tile offsets.
- `collect()` walks to all currently visible collectables and picks them up.
- `sleep(ms)` blocks the script for the given milliseconds.
- `sendPacket(packet)` sends a raw packet built from a Lua table. The table must include string `ID`.

### State and queries

```lua
bot:getPosition()
bot:getCurrentWorld()
bot:getStatus()
bot:isInWorld()
bot:getInventoryCount(block_id)
bot:getCollectables()
bot:isTileReadyToHarvest(x, y)
bot:getWorld()
```

Return values:

- `bot:getPosition()` returns:

```lua
{
  map_x = number_or_nil,
  map_y = number_or_nil,
  world_x = number_or_nil,
  world_y = number_or_nil
}
```

- `bot:getCurrentWorld()` returns the current world name or `nil`.
- `bot:getStatus()` returns the current session status as one of:
  `idle`, `connecting`, `authenticating`, `menu_ready`, `joining_world`, `loading_world`, `awaiting_ready`, `in_world`, `redirecting`, `disconnected`, or `error`.
- `bot:isInWorld()` returns `true` or `false`.
- `bot:getInventoryCount(block_id)` returns the summed amount for that block ID.
- `bot:getCollectables()` returns a Lua array of:

```lua
{
  id = number,
  block_type = number,
  amount = number,
  inventory_type = number,
  pos_x = number,
  pos_y = number,
  is_gem = boolean
}
```

- `bot:isTileReadyToHarvest(x, y)` returns `true` when the tracked grow state says the tile is ready.

## World Object

`bot:getWorld()` returns a world table with this shape:

```lua
local world = bot:getWorld()

world.name
world.width
world.height
world.spawn
world.tiles
world.objects
world:getTile(x, y)
world:isTileReadyToHarvest(x, y)
```

### `world.spawn`

```lua
{
  map_x = number_or_nil,
  map_y = number_or_nil,
  world_x = number_or_nil,
  world_y = number_or_nil
}
```

### `world.tiles`

All tile layers are exposed as flat row-major arrays:

```lua
world.tiles.foreground
world.tiles.background
world.tiles.water
world.tiles.wiring
```

To compute an index manually:

```lua
local index = y * world.width + x + 1
local fg = world.tiles.foreground[index]
```

Lua arrays are 1-based, so add `1` when indexing.

### `world.objects`

Currently exposed dynamic objects:

```lua
world.objects.collectables
world.objects.growing_tiles
```

`world.objects.collectables` entries:

```lua
{
  id = number,
  block_type = number,
  amount = number,
  inventory_type = number,
  pos_x = number,
  pos_y = number,
  is_gem = boolean
}
```

`world.objects.growing_tiles` entries:

```lua
{
  x = number,
  y = number,
  block_id = number,
  growth_end_time = number,
  growth_duration_secs = number,
  mixed = boolean,
  harvest_seeds = number,
  harvest_blocks = number,
  harvest_gems = number,
  harvest_extra_blocks = number
}
```

### `world:getTile(x, y)`

Returns:

```lua
{
  foreground = number,
  background = number,
  water = number,
  wiring = number,
  ready_to_harvest = boolean
}
```

### `world:isTileReadyToHarvest(x, y)`

Returns `true` if the tile has tracked grow data and the current time is past its `growth_end_time`.

## Raw Packet Example

```lua
bot:sendPacket({
  ID = "HB",
  x = 42,
  y = 29
})
```

Supported Lua value types in packets:

- `nil`
- `boolean`
- `number`
- `string`
- nested Lua tables

## Example Scripts

### Basic script

```lua
bot:warp("START")
bot:sleep(500)
bot:talk("hello")
bot:sleep(500)
bot:walk(1, 0)
bot:punch(1, 0)
bot:collect()
```

### Read world tiles

```lua
local world = bot:getWorld()
local tile = world:getTile(42, 29)

bot:talk("fg=" .. tile.foreground .. " bg=" .. tile.background)
```

### Wait for harvest

```lua
local world = bot:getWorld()
local x, y = 42, 29

while not world:isTileReadyToHarvest(x, y) do
  bot:sleep(1000)
  world = bot:getWorld()
end

bot:punch(x - bot:getPosition().map_x, y - bot:getPosition().map_y)
bot:collect()
```

## Error Behavior

- If no world is loaded, `bot:getWorld()` raises a Lua error.
- Out-of-bounds tile access raises a Lua error.
- Invalid raw packet tables raise a Lua error.
- If a script is stopped while sleeping or during a cancellable host action, it exits early.
