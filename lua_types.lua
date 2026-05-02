---@meta

-------------------------------------------------------------------------------
-- Custom Types & Enums
-------------------------------------------------------------------------------

---@alias SessionStatus "idle" | "connecting" | "authenticating" | "menu_ready" | "joining_world" | "loading_world" | "awaiting_ready" | "in_world" | "redirecting" | "disconnected" | "error"

---@class Position
---@field map_x? number
---@field map_y? number
---@field world_x? number
---@field world_y? number

---@class Point
---@field x number
---@field y number

---@class Collectable
---@field id number
---@field block_type number
---@field amount number
---@field inventory_type number
---@field pos_x number
---@field pos_y number
---@field is_gem boolean

---@class GrowingTile
---@field x number
---@field y number
---@field block_id number
---@field growth_end_time number
---@field growth_duration_secs number
---@field mixed boolean
---@field harvest_seeds number
---@field harvest_blocks number
---@field harvest_gems number
---@field harvest_extra_blocks number

---@class TileData
---@field foreground number
---@field background number
---@field water number
---@field wiring number
---@field ready_to_harvest boolean

---@class WorldTiles
---@field foreground number[] Flat row-major array. Index = y * width + x + 1
---@field background number[] Flat row-major array. Index = y * width + x + 1
---@field water number[] Flat row-major array. Index = y * width + x + 1
---@field wiring number[] Flat row-major array. Index = y * width + x + 1

---@class WorldObjects
---@field collectables Collectable[]
---@field growing_tiles GrowingTile[]

---@class PacketBase
---@field ID string
---@field [string] any

-------------------------------------------------------------------------------
-- World Object
-------------------------------------------------------------------------------

---@class World
---@field name string
---@field width number
---@field height number
---@field spawn Position
---@field tiles WorldTiles
---@field objects WorldObjects
local World = {}

--- Gets the tile data at the specified map coordinates.
---@param x number
---@param y number
---@return TileData
function World:getTile(x, y) end

--- Returns true if the tile has tracked grow data and current time is past growth_end_time.
---@param x number
---@param y number
---@return boolean
function World:isTileReadyToHarvest(x, y) end


-------------------------------------------------------------------------------
-- Bot Object
-------------------------------------------------------------------------------

---@class Bot
local Bot = {}

--- Joins the target world and waits until the session is fully in that world.
---@param world string
function Bot:warp(world) end

--- Joins an instance/special world (e.g. NETHERWORLD). Requires a consumable scroll.
---@param world string
function Bot:warpInstance(world) end

--- Walks using relative tile offsets from the current player tile.
---@param dx number
---@param dy number
function Bot:walk(dx, dy) end

--- Moves to an absolute map tile.
---@param x number
---@param y number
function Bot:walkTo(x, y) end

--- Finds a path to absolute coordinates.
---@param x number
---@param y number
---@return Point[]
function Bot:findPath(x, y) end

--- Punches using relative tile offsets.
---@param dx number
---@param dy number
function Bot:punch(dx, dy) end

--- Places a block using relative tile offsets.
---@param dx number
---@param dy number
---@param block_id number
function Bot:place(dx, dy, block_id) end

--- Wears an item.
---@param block_id number
function Bot:wear(block_id) end

--- Removes a worn item.
---@param block_id number
function Bot:unwear(block_id) end

--- Sends a chat message.
---@param message string
function Bot:talk(message) end

--- Walks to all currently visible collectables and picks them up.
function Bot:collect() end

--- Blocks the script for the given milliseconds.
---@param ms number
function Bot:sleep(ms) end

--- Sends a raw packet. The packet must be a PacketBase with a string ID.
---@param packet PacketBase
function Bot:sendPacket(packet) end

--- Returns the current position of the bot.
---@return Position
function Bot:getPosition() end

--- Returns the current world name, or nil if not in a world.
---@return string?
function Bot:getCurrentWorld() end

--- Returns the current session status.
---@return SessionStatus
function Bot:getStatus() end

--- Returns whether the bot is currently in a world.
---@return boolean
function Bot:isInWorld() end

--- Returns the summed amount for a specific block ID in the inventory.
---@param block_id number
---@return number
function Bot:getInventoryCount(block_id) end

--- Returns an array of collectables currently visible to the bot.
---@return Collectable[]
function Bot:getCollectables() end

--- Returns true when the tracked grow state says the tile is ready.
---@param x number
---@param y number
---@return boolean
function Bot:isTileReadyToHarvest(x, y) end

--- Returns the current world object. Raises a Lua error if no world is loaded.
---@return World
function Bot:getWorld() end

-------------------------------------------------------------------------------
-- Global Instances
-------------------------------------------------------------------------------

--- The global bot instance for the active Moonlight session.
---@type Bot
bot = nil