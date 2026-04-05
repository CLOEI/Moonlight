import socket
import struct
import threading
import time
import json
import math
import heapq
from collections import Counter, deque
from pathlib import Path

from auth_client import (
    DEVICE_ID,
    SERVER_HOST,
    SERVER_PORT,
    decode_bson,
    encode_bson,
    extract_msg,
    fetch_jwt_device,
    fetch_jwt_email,
    make_gpd,
    make_vchk,
    recv_packet,
    wrap_packet,
)

# Binary-backed scheduler notes:
# - ST is scheduled every 1000 ms by NetworkClient$$Update.
# - The 5 s keepalive is part of the general network update loop.
# - In-world idle traffic is a cached empty mP, not a full movement packet.
# - Binary RE shows the real ready point is later than rOP. Auto-ready now
#   waits for the later rAI stage instead of sending RtP immediately on rOP.
ST_INTERVAL = 1.0
MENU_KEEPALIVE_INTERVAL = 5.0
WALK_IDLE_SETTLE_INTERVAL = 0.12
WALK_STEP_INTERVAL = 0.18
PUNCH_IDLE_SETTLE_INTERVAL = 1.0
FRAME_SLEEP = 0.05
FISH_GAUGE_ON_DELAY = 0.20
FISH_GAUGE_OFF_DELAY = 0.45
FISH_GAUGE_CYCLE = 0.65
FISH_REARM_DELAY = 1.25
FISH_MAX_SIZE_MULTIPLIER = 2.8
FISH_MAX_DIFFICULTY_METER = 3.1
KEEPALIVE_ID = "p"
LEAVE_WORLD_ID = "LW"

DIR_RIGHT = 3
DIR_LEFT = 7
ANIM_IDLE = 1
ANIM_WALK = 2
ANIM_PUNCH = 6

# Binary-backed coordinate conversion:
# - WorldController$$ConvertPlayerMapPointToWorldPoint uses:
#     world_x = map_x * tile_width
#     world_y = map_y * tile_height - 0.5 * tile_height
# - The live values seen at spawn/hit alignment (map 40,30 -> world 12.8,9.44)
#   imply tile_width=0.32 and tile_height=0.32 for the current world controller.
TILE_WIDTH = 0.32
TILE_HEIGHT = 0.32

FISH_SIZE_BUCKETS = {
    "tiny": {
        "size": 1,
        "run_frequency": 0.02,
        "fish_move_speed": 0.60,
        "pull_strengths": {
            "bamboo": 1.7,
            "fiberglass": 1.175,
            "carbon": 0.775,
            "titanium": 0.5,
        },
        "min_land_delay": 4.2,
    },
    "small": {
        "size": 2,
        "run_frequency": 0.04,
        "fish_move_speed": 0.80,
        "pull_strengths": {
            "bamboo": 3.4,
            "fiberglass": 2.35,
            "carbon": 1.55,
            "titanium": 1.0,
        },
        "min_land_delay": 4.8,
    },
    "medium": {
        "size": 3,
        "run_frequency": 0.06,
        "fish_move_speed": 1.25,
        "pull_strengths": {
            "bamboo": 5.1,
            "fiberglass": 3.525,
            "carbon": 2.325,
            "titanium": 1.5,
        },
        "min_land_delay": 5.5,
    },
    "large": {
        "size": 4,
        "run_frequency": 0.08,
        "fish_move_speed": 1.90,
        "pull_strengths": {
            "bamboo": 6.8,
            "fiberglass": 4.7,
            "carbon": 3.1,
            "titanium": 2.0,
        },
        "min_land_delay": 6.4,
    },
    "giant": {
        "size": 5,
        "run_frequency": 0.10,
        "fish_move_speed": 2.50,
        "pull_strengths": {
            "bamboo": 8.5,
            "fiberglass": 5.875,
            "carbon": 3.875,
            "titanium": 2.5,
        },
        "min_land_delay": 7.2,
    },
}

ROD_FILL_MULTIPLIER = {
    2406: 1.2, 2410: 1.2, 2414: 1.2, 2418: 1.2,
    2407: 1.5, 2411: 1.5, 2415: 1.5, 2419: 1.5,
    2408: 2.1, 2412: 2.1, 2416: 2.1, 2420: 2.1,
    2409: 1.8, 2413: 1.8, 2417: 1.8, 2421: 1.8,
    4196: 2.1, 4622: 2.1,
}

ROD_MOVE_MULTIPLIER = {
    2406: 2.0, 2410: 2.0, 2414: 2.0, 2418: 2.0,
    2407: 2.3, 2411: 2.3, 2415: 2.3, 2419: 2.3,
    2408: 2.9, 2412: 2.9, 2416: 2.9, 2420: 2.9,
    2409: 2.6, 2413: 2.6, 2417: 2.6, 2421: 2.6,
    4196: 2.9, 4622: 2.9,
}

ROD_SIZE_MULTIPLIER = {
    2406: 1.0, 2407: 1.25, 2408: 1.5, 2409: 1.8,
    2410: 1.0, 2411: 1.25, 2412: 1.5, 2413: 1.8,
    2414: 1.0, 2415: 1.25, 2416: 1.5, 2417: 1.8,
    2418: 1.0, 2419: 1.25, 2420: 1.5, 2421: 1.8,
    4196: 1.5, 4622: 1.5,
}


def _csharp_ticks() -> int:
    return int(time.time() * 10_000_000) + 621_355_968_000_000_000


def _load_block_type_names(json_path: str = "block_types.json") -> dict[int, str]:
    path = Path(json_path)
    if not path.exists():
        return {}

    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}

    names = {}
    for key, value in raw.items():
        try:
            names[int(key)] = str(value)
        except (TypeError, ValueError):
            continue
    return names


def _normalize_name(value: str) -> str:
    return "".join(ch.lower() for ch in value if ch.isalnum())


class GameSession:
    def __init__(self, jwt: str, device_id: str, auto_ready: bool = True):
        self.jwt = jwt
        self.device_id = device_id
        self.auto_ready = auto_ready

        self.sock = None
        self.running = False

        self.current_world = None
        self._pending_join_world = None
        self._awaiting_world_data = False
        self._awaiting_spawn_ack = False
        self._awaiting_ready = False
        self._pending_spawn_setup = False
        self._spawn_packet_due = False
        self._in_world = False
        self._spawn_report_sent = False
        self.profile_gpd = {}
        self.profile_pd = {}
        self.players = {}
        self.world_tile_counts = Counter()
        self.world_width = 0
        self.world_height = 0
        self.world_tiles = ()
        self.world_collectables = []
        self.world_items = {}
        self.block_type_names = _load_block_type_names()
        self.current_host = SERVER_HOST
        self.current_port = SERVER_PORT
        self._reconnecting = False
        self._redirect_pending = None

        self.map_x = 40.0
        self.map_y = 30.0
        self.pos_x = 0.0
        self.pos_y = 0.0
        self.move_dir = DIR_RIGHT
        self.move_anim = ANIM_IDLE
        self._movement_dirty = False
        self._map_point_dirty = False
        self._teleport_pending = False
        self._move_pending = deque()

        self._outbox = []
        self._dedupe_keys = set()

        self._next_st_at = 0.0
        self._next_keepalive_at = 0.0
        self._next_move_step_at = 0.0
        self._next_walk_idle_at = 0.0
        self._next_punch_idle_at = 0.0
        self._last_send_at = 0.0
        self._st_count = 0
        self._walk_idle_due = False
        self._punch_idle_due = False

        self._recv_thread = None
        self._cmd_thread = None
        self._fishing_rearm_at = 0.0
        self._fishing_cleanup_pending = False
        self._fishing_loop = {
            "enabled": False,
            "pending_recast": False,
            "direction": None,
            "bait_text": "",
        }
        self._reset_fishing_state()
        self._update_world_position()

    def _reset_fishing_state(self):
        self._fishing = {
            "active": False,
            "direction": None,
            "bait_key": None,
            "bait_block": None,
            "target_x": None,
            "target_y": None,
            "cast_at": 0.0,
            "awaiting_hook": False,
            "hook_prompt_at": 0.0,
            "hook_sent": False,
            "hooked_at": 0.0,
            "fish_block": None,
            "rod_block": None,
            "gauge_started": False,
            "gauge_detected": False,
            "gauge_entered_at": 0.0,
            "next_gauge_step_at": 0.0,
            "gauge_overlap_on": False,
            "land_sent": False,
            "land_due_at": 0.0,
            "stop_sent": False,
            "instant_reward_confirmed": False,
            "sim_last_at": 0.0,
            "sim_fish_position": 0.5,
            "sim_target_position": 0.5,
            "sim_progress": 0.5,
            "sim_overlap_threshold": 0.12,
            "sim_fill_rate": 0.12,
            "sim_target_speed": 0.40,
            "sim_fish_move_speed": 0.80,
            "sim_run_frequency": 0.04,
            "sim_pull_strength": 3.4,
            "sim_slider_size": 1.0,
            "sim_fill_multiplier": 1.2,
            "sim_min_land_delay": 4.8,
            "sim_phase": 0.0,
            "sim_overlap": False,
            "sim_ready_to_land": False,
            "sim_ready_since": 0.0,
            "sim_off_distance": 0.0,
            "sim_difficulty_meter": 0.0,
            "sim_size_multiplier": 0.0,
            "sim_drag_extra": 0.5,
            "sim_run_active": False,
            "sim_run_until": 0.0,
            "sim_force_land_after": 0.0,
        }

    def _update_world_position(self):
        self.pos_x = float(self.map_x) * TILE_WIDTH
        self.pos_y = float(self.map_y) * TILE_HEIGHT - (0.5 * TILE_HEIGHT)

    def _update_profile_from_gpd(self, gpd: dict):
        self.profile_gpd = dict(gpd)
        pd = gpd.get("pD", {})
        if isinstance(pd, (bytes, bytearray)):
            try:
                pd = decode_bson(pd)
            except Exception:
                pd = {}
        self.profile_pd = pd if isinstance(pd, dict) else {}

    def _local_uid(self) -> str:
        uid = self.profile_gpd.get("U", "")
        return str(uid) if uid else ""

    def _packet_uid(self, msg: dict) -> str:
        uid = msg.get("U", "")
        return str(uid) if uid else ""

    def _should_log_other_player_packet(self, mid: str, msg: dict) -> bool:
        local_uid = self._local_uid()
        if not local_uid:
            return True

        packet_uid = self._packet_uid(msg)
        if packet_uid:
            return packet_uid == local_uid

        if mid == "WCM":
            cmb = msg.get("CmB", {})
            if isinstance(cmb, dict):
                chat_uid = str(cmb.get("userID", "") or "")
                if chat_uid:
                    return chat_uid == local_uid

        return True

    def _iter_inventory_entries(self):
        inv_blob = self.profile_pd.get("inv")
        if not isinstance(inv_blob, (bytes, bytearray)):
            return []
        if len(inv_blob) % 6 != 0:
            return []

        entries = []
        for offset in range(0, len(inv_blob), 6):
            inventory_key, amount = struct.unpack_from("<IH", inv_blob, offset)
            if amount <= 0:
                continue
            block_id = inventory_key & 0xFFFF
            inventory_type = (inventory_key >> 16) & 0xFFFF
            item_name = self.block_type_names.get(block_id, "<unknown>")
            entries.append((inventory_key, block_id, inventory_type, amount, item_name))

        entries.sort(key=lambda item: (-item[3], item[1], item[2], item[0]))
        return entries

    def _adjust_inventory_amount(self, inventory_key: int, delta: int) -> int | None:
        inv_blob = self.profile_pd.get("inv")
        if not isinstance(inv_blob, (bytes, bytearray)):
            return None
        if len(inv_blob) % 6 != 0:
            return None

        mutable = bytearray(inv_blob)
        for offset in range(0, len(mutable), 6):
            key, amount = struct.unpack_from("<IH", mutable, offset)
            if key != inventory_key:
                continue

            new_amount = max(0, amount + delta)
            struct.pack_into("<IH", mutable, offset, key, new_amount)
            self.profile_pd["inv"] = bytes(mutable)
            return new_amount

        return None

    def _add_inventory_entry(self, inventory_key: int, amount: int = 1) -> bool:
        if amount <= 0:
            return False
        inv_blob = self.profile_pd.get("inv")
        if not isinstance(inv_blob, (bytes, bytearray)):
            return False
        if len(inv_blob) % 6 != 0:
            return False

        mutable = bytearray(inv_blob)
        mutable.extend(struct.pack("<IH", int(inventory_key), int(amount)))
        self.profile_pd["inv"] = bytes(mutable)
        return True

    def _inventory_slot_limit(self) -> int:
        try:
            value = int(self.profile_pd.get("slots", 70))
        except (TypeError, ValueError):
            value = 70
        return max(1, value)

    def _inventory_used_slots(self) -> int:
        return len(self._iter_inventory_entries())

    def _inventory_is_full(self) -> bool:
        return self._inventory_used_slots() >= self._inventory_slot_limit()

    def _inventory_key_for_reward(self, reward_value: object) -> int | None:
        if not isinstance(reward_value, int):
            return None
        if reward_value <= 0:
            return None
        if reward_value > 0xFFFF:
            return int(reward_value)
        return int(reward_value & 0xFFFF)

    def _apply_reward_to_inventory(self, reward_value: object) -> bool:
        inventory_key = self._inventory_key_for_reward(reward_value)
        if inventory_key is None:
            return False

        updated = self._adjust_inventory_amount(inventory_key, 1)
        if updated is not None:
            return True
        return self._add_inventory_entry(inventory_key, 1)

    def _stop_fishing_loop(self, reason: str):
        if self._fishing_loop["enabled"]:
            print(f"    Auto-fish stopped: {reason}")
        self._fishing_loop["enabled"] = False
        self._fishing_loop["pending_recast"] = False
        self._fishing_loop["direction"] = None
        self._fishing_loop["bait_text"] = ""

    def _schedule_fishing_recast(self, delay: float = FISH_REARM_DELAY):
        self._fishing_rearm_at = time.time() + max(0.0, float(delay))
        if self._fishing_loop["enabled"]:
            self._fishing_loop["pending_recast"] = True

    def _is_fishing_lure_entry(self, entry) -> bool:
        if not entry or len(entry) < 5:
            return False
        _, _, inventory_type, amount, item_name = entry
        if int(amount or 0) <= 0:
            return False
        if int(inventory_type or 0) != 1792:
            return False
        return _normalize_name(str(item_name or "")).startswith("lure")

    def _next_available_lure_entry(self):
        for entry in self._iter_inventory_entries():
            if self._is_fishing_lure_entry(entry):
                return entry
        return None

    def _maybe_start_next_fishing_attempt(self):
        if not self._fishing_loop["enabled"] or not self._fishing_loop["pending_recast"]:
            return
        if self._fishing["active"] or self._fishing_cleanup_pending:
            return
        if time.time() < self._fishing_rearm_at:
            return
        if self._inventory_is_full():
            self._stop_fishing_loop(
                f"inventory full ({self._inventory_used_slots()}/{self._inventory_slot_limit()} slots)"
            )
            return

        bait_text = str(self._fishing_loop["bait_text"] or "").strip()
        direction = self._fishing_loop["direction"]
        bait_entry, error = self._resolve_bait_entry(bait_text)
        if error or bait_entry is None or bait_entry[3] <= 0:
            next_lure = self._next_available_lure_entry()
            if next_lure is None:
                self._stop_fishing_loop("no fishing lures remain in inventory")
                return
            bait_entry = next_lure
            bait_text = str(bait_entry[4])
            self._fishing_loop["bait_text"] = bait_text
            print(
                f"    Auto-fish switching lure -> {bait_entry[4]} "
                f"(block={bait_entry[1]}, key=0x{bait_entry[0]:08X}, amount={bait_entry[3]})"
            )

        self._fishing_loop["pending_recast"] = False
        self._queue_fish_cast(str(direction), bait_text, enable_loop=True)

    def _resolve_bait_entry(self, bait_text: str):
        query = bait_text.strip()
        if not query:
            return None, "Bait is required."

        entries = self._iter_inventory_entries()
        if not entries:
            return None, "No decoded inventory entries found yet."

        try:
            if query.lower().startswith("0x"):
                wanted = int(query, 16)
                for entry in entries:
                    if entry[0] == wanted:
                        return entry, None
                return None, f"No inventory item with key {query}."

            wanted = int(query)
            exact_block_matches = [entry for entry in entries if entry[1] == wanted]
            if exact_block_matches:
                return exact_block_matches[0], None
        except ValueError:
            pass

        normalized_query = _normalize_name(query)
        if not normalized_query:
            return None, "Bait is required."

        exact_name_matches = [
            entry for entry in entries if _normalize_name(entry[4]) == normalized_query
        ]
        if exact_name_matches:
            return exact_name_matches[0], None

        partial_matches = [
            entry for entry in entries if normalized_query in _normalize_name(entry[4])
        ]
        if len(partial_matches) == 1:
            return partial_matches[0], None
        if len(partial_matches) > 1:
            options = ", ".join(
                f"{entry[4]}(block={entry[1]}, key=0x{entry[0]:08X})"
                for entry in partial_matches[:5]
            )
            return None, f"Ambiguous bait '{query}'. Matches: {options}"

        return None, f"No bait match for '{query}'."

    def _resolve_block_name_or_id(self, item_text: str):
        query = item_text.strip()
        if not query:
            return None, "Item is required."

        try:
            block_id = int(query, 16) if query.lower().startswith("0x") else int(query)
            item_name = self.block_type_names.get(block_id, "<unknown>")
            return (block_id, item_name), None
        except ValueError:
            pass

        normalized_query = _normalize_name(query)
        if not normalized_query:
            return None, "Item is required."

        exact_matches = [
            (block_id, name)
            for block_id, name in self.block_type_names.items()
            if _normalize_name(name) == normalized_query
        ]
        if exact_matches:
            exact_matches.sort(key=lambda item: item[0])
            return exact_matches[0], None

        partial_matches = [
            (block_id, name)
            for block_id, name in self.block_type_names.items()
            if normalized_query in _normalize_name(name)
        ]
        if len(partial_matches) == 1:
            return partial_matches[0], None
        if len(partial_matches) > 1:
            partial_matches.sort(key=lambda item: item[0])
            options = ", ".join(f"{name}(block={block_id})" for block_id, name in partial_matches[:5])
            return None, f"Ambiguous item '{query}'. Matches: {options}"

        return None, f"No block match for '{query}'."

    def _queue_fishing_hook(self):
        if not self._fishing["active"] or self._fishing["hook_sent"]:
            return
        print("[>] FISH HOOK -> MGA(LS=2)")
        self._fishing["hook_sent"] = True
        self._fishing["awaiting_hook"] = False
        self._fishing["hooked_at"] = time.time()
        self._queue_message({"ID": "MGA", "MGT": 2, "MGD": _csharp_ticks(), "LS": 2})

    def _rod_family_name(self, rod_block: int | None) -> str:
        rod = int(rod_block or 2406)
        rod_index = (rod - 2406) % 4 if 2406 <= rod <= 2421 else None
        if rod_index == 0:
            return "bamboo"
        if rod_index == 1:
            return "fiberglass"
        if rod_index == 2:
            return "carbon"
        if rod_index == 3:
            return "titanium"
        return "bamboo"

    def _resolve_fish_bucket(self) -> dict:
        fish_name = self.block_type_names.get(int(self._fishing["fish_block"] or 0), "")
        normalized = _normalize_name(fish_name)
        if "tiny" in normalized:
            return FISH_SIZE_BUCKETS["tiny"]
        if "small" in normalized:
            return FISH_SIZE_BUCKETS["small"]
        if "medium" in normalized:
            return FISH_SIZE_BUCKETS["medium"]
        if "large" in normalized:
            return FISH_SIZE_BUCKETS["large"]
        return FISH_SIZE_BUCKETS["giant"] if "giant" in normalized else FISH_SIZE_BUCKETS["small"]

    def _init_fishing_gauge_profile(self):
        rod_block = int(self._fishing["rod_block"] or 2406)
        fish_bucket = self._resolve_fish_bucket()
        rod_family = self._rod_family_name(rod_block)
        fill_multiplier = ROD_FILL_MULTIPLIER.get(rod_block, 1.2)
        slider_speed = ROD_MOVE_MULTIPLIER.get(rod_block, 2.0)
        slider_size = ROD_SIZE_MULTIPLIER.get(rod_block, 1.0)

        self._fishing.update(
            {
                "sim_overlap_threshold": 0.095 + (slider_size * 0.035),
                "sim_fill_rate": fill_multiplier * 0.10,
                "sim_target_speed": slider_speed * 0.20,
                "sim_fish_move_speed": fish_bucket["fish_move_speed"],
                "sim_run_frequency": fish_bucket["run_frequency"],
                "sim_pull_strength": fish_bucket["pull_strengths"][rod_family],
                "sim_slider_size": slider_size,
                "sim_fill_multiplier": fill_multiplier,
                "sim_min_land_delay": fish_bucket["min_land_delay"],
            }
        )
        self._fishing["sim_last_at"] = time.time()
        self._fishing["sim_fish_position"] = 0.5
        self._fishing["sim_target_position"] = 0.5
        self._fishing["sim_progress"] = 0.5
        self._fishing["sim_phase"] = 0.0
        self._fishing["sim_overlap"] = False
        self._fishing["sim_ready_to_land"] = False
        self._fishing["sim_ready_since"] = 0.0
        self._fishing["sim_off_distance"] = 0.0
        self._fishing["sim_difficulty_meter"] = 0.0
        self._fishing["sim_size_multiplier"] = 0.0
        self._fishing["sim_drag_extra"] = 1.0
        self._fishing["sim_run_active"] = False
        self._fishing["sim_run_until"] = 0.0
        self._fishing["sim_force_land_after"] = time.time() + max(
            fish_bucket["min_land_delay"] + 4.0,
            9.0,
        )

    def _current_fishing_land_values(self) -> tuple[int, int, float]:
        size_multiplier = max(
            0.001,
            min(float(self._fishing["sim_size_multiplier"]), FISH_MAX_SIZE_MULTIPLIER),
        )
        difficulty_meter = max(
            0.001,
            min(float(self._fishing["sim_difficulty_meter"]), FISH_MAX_DIFFICULTY_METER),
        )
        v_i = max(1, int(size_multiplier * 1000.0))
        idx = max(1, int(difficulty_meter * 1000.0))
        amt = self._fishing["sim_fish_position"] - self._fishing["sim_drag_extra"]
        return v_i, idx, amt

    def _queue_fishing_land(self, include_metrics: bool):
        if not self._fishing["active"] or self._fishing["land_sent"]:
            return

        msg = {"ID": "MGA", "MGT": 2, "MGD": _csharp_ticks(), "LS": 1}
        if include_metrics:
            v_i, idx, amt = self._current_fishing_land_values()
            msg["vI"] = v_i
            msg["Idx"] = idx
            msg["Amt"] = amt
            print(
                "[>] FISH LAND -> MGA(LS=1) "
                f"vI={v_i} Idx={idx} Amt={amt} fish={self._fishing['fish_block']}"
            )
        else:
            print("[>] FISH LAND -> MGA(LS=1) instant/special path")

        self._fishing["land_sent"] = True
        self._queue_message(msg)

    def _queue_fishing_cleanup(self):
        # The real client has an observed reset/exit path using MGA(MGD=1, LS=0)
        # that leads into MGSp cleanup. Sending it after MGC helps re-arm
        # fishing for the next cast in the headless flow.
        print("[>] FISH CLEANUP -> MGA(MGD=1, LS=0)")
        self._fishing_cleanup_pending = True
        self._queue_message({"ID": "MGA", "MGT": 2, "MGD": 1, "LS": 0})

    def _queue_fish_cast(self, direction: str, bait_text: str, enable_loop: bool = True):
        if not self._in_world:
            print("    Not in a world yet.")
            return

        now = time.time()
        if self._fishing_cleanup_pending:
            print("    Fishing cleanup still pending. Wait for cleanup ack before recasting.")
            return
        if now < self._fishing_rearm_at:
            wait_left = self._fishing_rearm_at - now
            print(f"    Fishing re-arm cooldown active for {wait_left:.1f}s.")
            return

        if direction not in ("left", "right"):
            print("    Usage: fish <left|right> <bait>")
            return

        if self._inventory_is_full():
            print(
                f"    Inventory full ({self._inventory_used_slots()}/{self._inventory_slot_limit()} slots)."
            )
            return

        bait_entry, error = self._resolve_bait_entry(bait_text)
        if error:
            print(f"    {error}")
            return

        inventory_key, block_id, inventory_type, amount, item_name = bait_entry
        base_x = int(round(self.map_x))
        base_y = int(round(self.map_y))
        target_x = base_x + (-1 if direction == "left" else 1)
        target_y = base_y - 1

        self.move_dir = DIR_LEFT if direction == "left" else DIR_RIGHT
        self._fishing_loop["enabled"] = bool(enable_loop)
        self._fishing_loop["pending_recast"] = False
        self._fishing_loop["direction"] = direction
        self._fishing_loop["bait_text"] = bait_text
        self._reset_fishing_state()
        self._fishing.update(
            {
                "active": True,
                "direction": direction,
                "bait_key": int(inventory_key),
                "bait_block": int(block_id),
                "target_x": int(target_x),
                "target_y": int(target_y),
                "cast_at": time.time(),
                "awaiting_hook": True,
            }
        )

        print(
            f"[>] FISH {direction} bait={item_name} "
            f"key=0x{inventory_key:08X} block={block_id} inv={inventory_type} "
            f"target=({target_x}, {target_y})"
        )
        remaining = self._adjust_inventory_amount(int(inventory_key), -1)
        if remaining is not None:
            print(f"    Local bait count -> {remaining}")
        self._queue_messages(
            [
                {"ID": "BUp", "Bi": int(inventory_key)},
                {"ID": "TrTFFMP", "BT": int(block_id), "x": int(target_x), "y": int(target_y)},
                {"ID": "MGSt", "MGT": 2, "x": int(target_x), "y": int(target_y), "BT": int(block_id)},
            ]
        )

    def _queue_wearable_change(self, action: str, item_text: str):
        if not self._in_world:
            print("    Not in a world yet.")
            return

        resolved, error = self._resolve_block_name_or_id(item_text)
        if error:
            print(f"    {error}")
            return

        block_id, item_name = resolved
        message_id = "WeOwC" if action == "wear" else "WeOwU"
        print(f"[>] {message_id} block={block_id} name={item_name}")
        self._queue_messages([{"ID": "mP"}, {"ID": message_id, "hBlock": int(block_id)}])

    def _queue_message(self, msg: dict, dedupe_key: str | None = None):
        if dedupe_key and dedupe_key in self._dedupe_keys:
            return
        self._outbox.append(msg)
        if dedupe_key:
            self._dedupe_keys.add(dedupe_key)

    def _queue_messages(self, messages: list[dict]):
        for msg in messages:
            self._queue_message(msg)

    def _flush_outbox(self) -> bool:
        if not self._outbox:
            return False
        if self.sock is None:
            return False

        doc = {f"m{i}": msg for i, msg in enumerate(self._outbox)}
        doc["mc"] = len(self._outbox)
        bson_data = encode_bson(doc)
        packet = struct.pack("<I", len(bson_data) + 4) + bson_data
        self.sock.sendall(packet)

        self._outbox.clear()
        self._dedupe_keys.clear()
        self._last_send_at = time.time()
        return True

    def _send_and_wait(self, messages: list[dict]) -> dict:
        self._queue_messages(messages)
        self._flush_outbox()
        return recv_packet(self.sock)

    def _queue_st(self):
        self._queue_message({"ID": "ST", "T": _csharp_ticks()}, dedupe_key="st")

    def _queue_keepalive(self):
        self._queue_message({"ID": KEEPALIVE_ID}, dedupe_key="menu_keepalive")

    def _queue_map_point_update(self, include_ping: bool = False):
        chunk_x = int(round(self.map_x))
        chunk_y = int(round(self.map_y))
        self._queue_message({"ID": "mp", "pM": struct.pack("<ii", chunk_x, chunk_y)})
        if include_ping:
            self._queue_message({"ID": KEEPALIVE_ID})

    def _queue_spawn_packet(self):
        self._queue_map_point_update(include_ping=False)
        self._queue_message(
            {
                "ID": "mP",
                "x": float(self.pos_x),
                "y": float(self.pos_y),
                "t": _csharp_ticks(),
                "a": ANIM_IDLE,
                "d": self.move_dir,
                "tp": True,
            }
        )
        self._spawn_report_sent = True
        self._teleport_pending = False
        self._map_point_dirty = False

    def _queue_full_mp(self):
        msg = {
            "ID": "mP",
            "x": float(self.pos_x),
            "y": float(self.pos_y),
            "t": _csharp_ticks(),
            "a": self.move_anim,
            "d": self.move_dir,
        }
        if self._teleport_pending:
            msg["tp"] = True
            self._teleport_pending = False
            self._spawn_report_sent = True
        self._queue_message(msg)

    def _queue_world_chat(self, text: str):
        message = text.strip()
        if not message:
            print("    Chat text is empty.")
            return
        if not self._in_world:
            print("    Not in a world yet.")
            return

        print(f"[>] WCM {message!r}")
        self._queue_messages(
            [
                {"ID": "mP"},
                {"ID": "WCM", "msg": message},
                {"ID": "PSicU", "SIc": 0},
            ]
        )

    def _queue_hit(self, direction: str):
        if not self._in_world:
            print("    Not in a world yet.")
            return

        offsets = {
            "left": (-1, 0),
            "right": (1, 0),
            "up": (0, 1),
            "down": (0, -1),
        }
        dx, dy = offsets[direction]
        target_x = int(round(self.map_x + dx))
        target_y = int(round(self.map_y + dy))

        if direction == "left":
            self.move_dir = DIR_LEFT
        elif direction == "right":
            self.move_dir = DIR_RIGHT

        print(f"[>] HB {direction} -> ({target_x}, {target_y})")
        self.move_anim = ANIM_PUNCH
        self._punch_idle_due = True
        self._next_punch_idle_at = time.time() + PUNCH_IDLE_SETTLE_INTERVAL
        self._queue_messages(
            [
                {
                    "ID": "mP",
                    "x": float(self.pos_x),
                    "y": float(self.pos_y),
                    "t": _csharp_ticks(),
                    "a": ANIM_PUNCH,
                    "d": self.move_dir,
                },
                {"ID": "HB", "x": target_x, "y": target_y},
            ]
        )

    def _queue_place_block(self, target_x: int, target_y: int, block_id: int):
        if not self._in_world:
            print("    Not in a world yet.")
            return

        print(f"[>] SB ({target_x}, {target_y}) block={block_id}")
        self._queue_message(
            {
                "ID": "SB",
                "x": int(target_x),
                "y": int(target_y),
                "BlockType": int(block_id),
            }
        )

    def connect_and_auth(self) -> bool:
        print(f"[1] Connecting to {self.current_host}:{self.current_port} ...")
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.sock.connect((self.current_host, self.current_port))
        print("    Connected.")

        self.sock.sendall(wrap_packet(make_vchk(self.device_id)))
        vchk = extract_msg(recv_packet(self.sock))
        if vchk.get("ID") != "VChk" or vchk.get("VN") != 200:
            print(f"[!] VChk rejected: {vchk}")
            return False
        print("    Version OK (VN=200)")

        self.sock.sendall(wrap_packet(make_gpd(self.jwt)))
        gpd = extract_msg(recv_packet(self.sock))
        if gpd.get("ID") != "GPd":
            print(f"[!] Auth failed: {gpd}")
            return False
        self._update_profile_from_gpd(gpd)
        print(f"[<] GPd: {gpd}")
        print(f"    Authenticated as: {gpd.get('UN')}  (sub={gpd.get('U')})")

        print("    [Handshake] Initial ST clock sync ...")
        for _ in range(2):
            self._send_and_wait([{"ID": "ST", "T": _csharp_ticks()}])

        print("    [Handshake] Transitioning to #menu ...")
        self._send_and_wait(
            [
                {"ID": "WREU", "WREgA": True},
                {"ID": "BcsU"},
                {"ID": "ULS", "LS": "#menu"},
                {"ID": "DailyBonusGiveAway"},
                {"ID": "ST", "T": _csharp_ticks()},
            ]
        )

        print("    [Handshake] gLSI ...")
        self._send_and_wait([{"ID": "gLSI"}, {"ID": "ST", "T": _csharp_ticks()}])

        print("    [Handshake] Complete.\n")

        now = time.time()
        self._last_send_at = now
        self._next_st_at = now + ST_INTERVAL
        self._next_keepalive_at = now + MENU_KEEPALIVE_INTERVAL
        return True

    def _reset_session_state_for_reconnect(self):
        self._outbox.clear()
        self._dedupe_keys.clear()
        self._pending_join_world = None
        self._awaiting_world_data = False
        self._awaiting_spawn_ack = False
        self._awaiting_ready = False
        self._pending_spawn_setup = False
        self._spawn_packet_due = False
        self._in_world = False
        self._spawn_report_sent = False
        self._movement_dirty = False
        self._map_point_dirty = False
        self._teleport_pending = False
        self._move_pending.clear()
        self._walk_idle_due = False
        self._next_move_step_at = 0.0
        self._punch_idle_due = False
        self.players.clear()
        self.world_tile_counts.clear()
        self.world_width = 0
        self.world_height = 0
        self.world_tiles = ()
        self.world_collectables.clear()
        self.world_items.clear()
        self._reset_fishing_state()

    def _reset_world_state_for_leave(self):
        self._pending_join_world = None
        self._awaiting_world_data = False
        self._awaiting_spawn_ack = False
        self._awaiting_ready = False
        self._pending_spawn_setup = False
        self._spawn_packet_due = False
        self._in_world = False
        self._spawn_report_sent = False
        self._movement_dirty = False
        self._map_point_dirty = False
        self._teleport_pending = False
        self._move_pending.clear()
        self._walk_idle_due = False
        self._next_move_step_at = 0.0
        self._punch_idle_due = False
        self.players.clear()
        self.world_tile_counts.clear()
        self.world_width = 0
        self.world_height = 0
        self.world_tiles = ()
        self.world_collectables.clear()
        self.world_items.clear()
        self._reset_fishing_state()
        self.current_world = None

        now = time.time()
        self._next_keepalive_at = now + MENU_KEEPALIVE_INTERVAL

    def _queue_leave_world(self):
        if not (
            self.current_world
            or self._in_world
            or self._awaiting_world_data
            or self._awaiting_spawn_ack
            or self._awaiting_ready
        ):
            print("    Not in a world.")
            return

        print(f"[>] {LEAVE_WORLD_ID}")
        self._outbox.clear()
        self._dedupe_keys.clear()
        self._queue_message({"ID": LEAVE_WORLD_ID})
        self._reset_world_state_for_leave()

    def _handle_other_owner_ip(self, host: str, world_name: str):
        redirect_host = host.strip()
        redirect_world = world_name.strip().upper()
        if not redirect_host or not redirect_world:
            print(f"[!] OoIP malformed: host={host!r} world={world_name!r}")
            return

        self._redirect_pending = (redirect_host, redirect_world)

    def _perform_pending_redirect(self):
        if not self._redirect_pending:
            return

        redirect_host, redirect_world = self._redirect_pending
        self._redirect_pending = None
        print(f"[>] Redirecting to {redirect_host}:{self.current_port} for {redirect_world}")
        self._reconnecting = True

        old_sock = self.sock
        self.sock = None
        if old_sock:
            try:
                old_sock.close()
            except OSError:
                pass

        self.current_host = redirect_host
        self.current_world = None
        self._reset_session_state_for_reconnect()

        if not self.connect_and_auth():
            print("[!] Redirect reconnect/auth failed.")
            self.running = False
            self._reconnecting = False
            return

        self.join_world(redirect_world)
        self._reconnecting = False

    def join_world(self, world_name: str):
        world = world_name.upper()
        self._pending_join_world = world
        self.players.clear()
        self.world_tile_counts.clear()
        print(f"[>] TTjW -> {world}")
        self._queue_message({"ID": "TTjW", "W": world, "WB": 0, "Amt": 0})

    def _queue_enter_world(self, world_name: str):
        print(f"[>] Gw -> {world_name}")
        self._awaiting_world_data = True
        self._queue_messages(
            [
                {"ID": "Gw", "eID": "", "W": world_name, "WB": 0},
                {"ID": "A", "AE": 2},
                {"ID": "A", "AE": 6},
                {"ID": "A", "AE": 14},
                {"ID": "A", "AE": 23},
                {"ID": "GSb"},
            ]
        )

    def _queue_spawn_location_sync(self, world_name: str):
        print(f"[>] ULS/ST -> {world_name}")
        self._queue_messages([{"ID": "ULS", "LS": world_name}, {"ID": "ST", "T": _csharp_ticks()}])

    def _queue_world_spawn_setup(self, world_name: str):
        print(f"[>] Spawn setup -> {world_name}")
        self._awaiting_spawn_ack = True
        self._queue_messages(
            [
                {"ID": "cZL", "CZL": 2},
                {"ID": "cZva", "Amt": 0.5},
                {"ID": "rOP"},
                {"ID": "rAIp"},
                {"ID": "rAI"},
            ]
        )

    def _queue_ready_to_play(self):
        print("[>] RtP/ST")
        self._queue_messages([{"ID": "RtP"}, {"ID": "ST", "T": _csharp_ticks()}])
        self._awaiting_ready = False
        self._spawn_packet_due = True

    def _request_ready(self):
        if not self.current_world:
            print("    Not in a world join flow.")
            return
        if not self._awaiting_ready and not self.auto_ready:
            print("    No pending ready stage.")
            return
        self._queue_ready_to_play()

    def _apply_move(self, dx: float, dy: float):
        self.map_x += dx
        self.map_y += dy
        self._update_world_position()
        self._punch_idle_due = False
        if dx > 0:
            self.move_dir = DIR_RIGHT
        elif dx < 0:
            self.move_dir = DIR_LEFT

        if dx != 0.0 or dy != 0.0:
            self.move_anim = ANIM_WALK
            self._movement_dirty = True
            self._map_point_dirty = True
            self._walk_idle_due = True

    def _get_tile_id(self, tile_x: int, tile_y: int) -> int | None:
        if (
            self.world_width <= 0
            or self.world_height <= 0
            or not self.world_tiles
            or tile_x < 0
            or tile_y < 0
            or tile_x >= self.world_width
            or tile_y >= self.world_height
        ):
            return None
        return int(self.world_tiles[(tile_y * self.world_width) + tile_x])

    def _is_path_tile_passable(self, tile_x: int, tile_y: int, goal: tuple[int, int] | None = None) -> bool:
        tile_id = self._get_tile_id(tile_x, tile_y)
        if tile_id is None:
            return False
        if goal is not None and (tile_x, tile_y) == goal:
            return True
        return tile_id == 0

    def _find_path_astar(self, start: tuple[int, int], goal: tuple[int, int]) -> list[tuple[int, int]] | None:
        if not self.world_tiles or self.world_width <= 0 or self.world_height <= 0:
            return None

        if start == goal:
            return [start]

        if not self._is_path_tile_passable(goal[0], goal[1], goal=goal):
            return None

        open_heap: list[tuple[int, int, tuple[int, int]]] = []
        heapq.heappush(open_heap, (0, 0, start))
        came_from: dict[tuple[int, int], tuple[int, int]] = {}
        g_score: dict[tuple[int, int], int] = {start: 0}
        tie = 0

        while open_heap:
            _, _, current = heapq.heappop(open_heap)
            if current == goal:
                path = [current]
                while current in came_from:
                    current = came_from[current]
                    path.append(current)
                path.reverse()
                return path

            cx, cy = current
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                neighbor = (cx + dx, cy + dy)
                if not self._is_path_tile_passable(neighbor[0], neighbor[1], goal=goal):
                    continue

                tentative_g = g_score[current] + 1
                if tentative_g >= g_score.get(neighbor, 1 << 30):
                    continue

                came_from[neighbor] = current
                g_score[neighbor] = tentative_g
                heuristic = abs(goal[0] - neighbor[0]) + abs(goal[1] - neighbor[1])
                tie += 1
                heapq.heappush(open_heap, (tentative_g + heuristic, tie, neighbor))

        return None

    def _queue_walk_to(self, goal_x: int, goal_y: int):
        if not self._in_world:
            print("    Not in a world yet.")
            return

        if not self.world_tiles or self.world_width <= 0 or self.world_height <= 0:
            print("    No world grid loaded yet. Join a world first.")
            return

        start = (int(round(self.map_x)), int(round(self.map_y)))
        goal = (int(goal_x), int(goal_y))
        path = self._find_path_astar(start, goal)
        if not path:
            print(f"    No A* path found from {start} to {goal}.")
            return

        steps = 0
        for (x0, y0), (x1, y1) in zip(path, path[1:]):
            self._move_pending.append((float(x1 - x0), float(y1 - y0)))
            steps += 1

        print(f"[>] WALKTO {goal} path_len={len(path)} steps={steps}")

    def _recv_loop(self):
        while self.running:
            try:
                if self._reconnecting or self._redirect_pending or self.sock is None:
                    time.sleep(0.05)
                    continue
                self.sock.settimeout(1.0)
                try:
                    outer = recv_packet(self.sock)
                except socket.timeout:
                    continue

                mc = outer.get("mc", 0)
                for i in range(mc):
                    self._handle_message(outer.get(f"m{i}", {}))
            except Exception:
                import traceback

                if self.running and not self._reconnecting:
                    print("\n[!] Recv error:")
                    traceback.print_exc()
                    self.running = False
                    break

    def _handle_message(self, msg: dict):
        mid = msg.get("ID", "")

        if mid == "ST":
            return

        if mid == KEEPALIVE_ID:
            return

        if mid == "VChk":
            return

        if mid == "GPd":
            self._update_profile_from_gpd(msg)
            print(f"[<] GPd: {msg}")
            return

        if mid == "WREU":
            return

        if mid == "TrTFFMP" and self._fishing["active"]:
            if msg.get("S") is True:
                print(
                    f"    Fishing cast acknowledged at "
                    f"({msg.get('x')}, {msg.get('y')}) bait={msg.get('BT')}"
                )
            return

        if mid == "MGSt" and self._fishing["active"]:
            if msg.get("S") is True:
                print("    Fishing start acknowledged.")
            return

        if mid == "MGA":
            if self._fishing["active"] and msg.get("MGT") == 2:
                mgd = msg.get("MGD")
                if mgd == 2:
                    print("[<] Fishing hook prompt received.")
                    self._fishing["hook_prompt_at"] = time.time()
                    self._queue_fishing_hook()
                    return
                if mgd == 3:
                    fish_block = msg.get("BT")
                    rod_block = msg.get("WBT")
                    self._fishing["fish_block"] = fish_block
                    self._fishing["rod_block"] = rod_block
                    self._fishing["gauge_started"] = True
                    self._fishing["gauge_detected"] = True
                    self._fishing["gauge_entered_at"] = time.time()
                    self._fishing["next_gauge_step_at"] = time.time() + FISH_GAUGE_ON_DELAY
                    self._fishing["gauge_overlap_on"] = False
                    self._init_fishing_gauge_profile()
                    fish_name = self.block_type_names.get(int(fish_block or 0), "<unknown>")
                    rod_name = self.block_type_names.get(int(rod_block or 0), "<unknown>")
                    print(
                        f"[<] Fishing gauge/start payload fish={fish_block} ({fish_name}) "
                        f"rod={rod_block} ({rod_name})"
                    )
                    return
                if mgd == 1:
                    print("[<] Fishing MGA MGD=1 observed; current land attempt was rejected/reset.")
                    self._reset_fishing_state()
                    self._schedule_fishing_recast(0.75)
                    return
                if mgd == 5:
                    print("[<] Fishing MGA MGD=5 observed; treating as failure/reset signal.")
                    self._reset_fishing_state()
                    self._schedule_fishing_recast(0.75)
                    return

            print(f"[<] {mid}: {msg}")
            return

        if mid in {"FiOnAM", "FiOffAM", "FiRM"} and self._fishing["active"]:
            packet_uid = str(msg.get("U", "") or "")
            local_uid = self._local_uid()
            if packet_uid and local_uid and packet_uid != local_uid:
                return
            if mid == "FiOnAM":
                self._fishing["gauge_detected"] = True
                self._fishing["gauge_overlap_on"] = True
                self._fishing["next_gauge_step_at"] = time.time() + FISH_GAUGE_OFF_DELAY
            elif mid == "FiOffAM":
                self._fishing["gauge_detected"] = True
                self._fishing["gauge_overlap_on"] = False
                self._fishing["next_gauge_step_at"] = time.time() + FISH_GAUGE_ON_DELAY
            elif mid == "FiRM":
                self._fishing["gauge_detected"] = True
            print(f"[<] {mid}: {msg}")
            return

        if mid == "MGSp" and self._fishing["active"]:
            print(f"[<] MGSp: {msg}")
            self._reset_fishing_state()
            self._schedule_fishing_recast(0.75)
            return

        if mid == "MGSp" and self._fishing_cleanup_pending:
            print(f"[<] MGSp cleanup ack: {msg}")
            self._fishing_cleanup_pending = False
            self._schedule_fishing_recast(FISH_REARM_DELAY)
            return

        if mid == "MGC":
            print(f"[<] MGC: {msg}")
            reward_key = msg.get("IK")
            if self._apply_reward_to_inventory(reward_key):
                print(
                    f"    Local inventory reward applied. "
                    f"slots={self._inventory_used_slots()}/{self._inventory_slot_limit()}"
                )
            self._reset_fishing_state()
            self._queue_fishing_cleanup()
            return

        if mid == "TTjW":
            jr = msg.get("JR")
            if jr is None or jr != 0:
                err = msg.get("E", msg.get("Err", jr))
                print(f"[<] TTjW denied - {err}")
                self._pending_join_world = None
                return

            world = msg.get("WN", self._pending_join_world or "?")
            self.current_world = world
            print(f"[<] TTjW OK -> {world}")
            if self._pending_join_world:
                queued_world = self._pending_join_world
                self._pending_join_world = None
                self._queue_enter_world(queued_world)
            return

        if mid == "GWC":
            raw = msg.get("W", b"")
            size = len(raw) if isinstance(raw, (bytes, bytearray)) else 0
            print(f"[<] GWC ({size}B compressed)")
            self._process_gwc(raw)
            return

        if mid == "OoIP":
            redirect_host = msg.get("IP", "")
            redirect_world = msg.get("WN", self._pending_join_world or self.current_world or "")
            print(f"[<] OoIP: {msg}")
            self._handle_other_owner_ip(redirect_host, redirect_world)
            return

        if mid == "rOP":
            self._awaiting_spawn_ack = False
            self._awaiting_ready = True
            print("[<] rOP")
            if self.auto_ready:
                print("    Waiting for later ready signal before RtP/ST.")
            else:
                print("    Waiting for explicit 'ready' command.")
            return

        if mid == "rAI":
            print("[<] rAI")
            if self.auto_ready and self._awaiting_ready:
                print("    Queueing RtP/ST on rAI.")
                self._queue_ready_to_play()
            return

        if mid == "GRW":
            wn = msg.get("WN", [])
            ct = msg.get("Ct", [])
            print("\n[<] Recent Worlds:")
            for i in range(min(len(wn), 10)):
                players = ct[i] if i < len(ct) else "?"
                print(f"      {wn[i]:<20} players={players}")
            print()
            return

        if mid == "U":
            uid = msg.get("U")
            name = msg.get("UN", "")
            x = msg.get("x")
            y = msg.get("y")
            if uid and name:
                player = self.players.setdefault(uid, {})
                player["name"] = name
                player["x"] = x
                player["y"] = y
                if self._should_log_other_player_packet(mid, msg):
                    print(f"[<] Player: {name} @ ({x}, {y})")
            return

        if mid == "AnP":
            uid = msg.get("U")
            if uid:
                player = self.players.setdefault(uid, {})
                player["name"] = msg.get("UN", player.get("name", "?"))
                player["x"] = msg.get("x", player.get("x"))
                player["y"] = msg.get("y", player.get("y"))
                player["a"] = msg.get("a", player.get("a"))
                player["d"] = msg.get("d", player.get("d"))
                player["t"] = msg.get("t", player.get("t"))
                player["gender"] = msg.get("Gnd", player.get("gender"))
                player["skin"] = msg.get("skin", player.get("skin"))
                player["face_anim"] = msg.get("faceAnim", player.get("face_anim"))
                player["in_portal"] = msg.get("inPortal", player.get("in_portal"))
                player["sic"] = msg.get("SIc", player.get("sic"))
                player["spots"] = msg.get("spots", player.get("spots"))
                if self._should_log_other_player_packet(mid, msg):
                    print(f"[<] AnP: {player.get('name', '?')} @ ({player.get('x')}, {player.get('y')})")
            return

        if mid == "mP":
            uid = msg.get("U")
            if uid:
                player = self.players.setdefault(uid, {})
                player["x"] = msg.get("x", player.get("x"))
                player["y"] = msg.get("y", player.get("y"))
                player["a"] = msg.get("a", player.get("a"))
                player["d"] = msg.get("d", player.get("d"))
                player["t"] = msg.get("t", player.get("t"))
            return

        if mid == "PL":
            uid = msg.get("U")
            if uid:
                player = self.players.pop(uid, None)
                if self._should_log_other_player_packet(mid, msg):
                    if player:
                        print(f"[<] PL: {player.get('name', '?')} left (uid={uid})")
                    else:
                        print(f"[<] PL: uid={uid}")
            return

        if mid == "AC":
            print("[!] AC - server sent Already Connected.")
            self.running = False
            return

        if mid == "BGM":
            cmb = msg.get("CmB", {})
            if isinstance(cmb, dict):
                nick = cmb.get("nick", "?")
                channel = cmb.get("channel", "?")
                text = cmb.get("message", "")
                print(f"[<] BGM [{channel}] {nick}: {text}")
            return

        if mid in {
            "PSicU",
            "PPA",
            "WCM",
            "FiOnAM",
            "FiOffAM",
            "FiRM",
            "FiBmS",
            "FiBmSt",
            "FiBmH",
            "FiBmM",
            "FiBmML",
            "FiBmMW",
            "WeOwC",
            "WeOwU",
            "HB",
            "HBB",
            "HBW",
            "FrRq",
            "USe",
            "TVC",
            "RLU",
            "HitEEgg",
        } and not self._should_log_other_player_packet(mid, msg):
            return

        if mid not in {
            "BcsU",
            "DailyBonusGiveAway",
            "gLSI",
            "GAW",
            "GWotW",
            "GFW",
            "MWli",
        }:
            print(f"[<] {mid}: {msg}")

    def _process_gwc(self, raw: bytes):
        try:
            import zstandard as zstd
        except ImportError:
            print("    [!] pip install zstandard needed for GWC decode")
            return

        try:
            world_bson = decode_bson(
                zstd.ZstdDecompressor().decompress(raw, max_output_size=64 * 1024 * 1024)
            )
        except Exception as exc:
            print(f"    [!] GWC decompress failed: {exc}")
            return

        world_name = self.current_world or "world"
        with open(f"{world_name}.bin", "wb") as handle:
            handle.write(encode_bson({"m0": {"ID": "GWC", "W": raw}, "mc": 1}))
        print(f"    Saved -> {world_name}.bin")

        spawn = world_bson.get("WorldStartPoint", {})
        if isinstance(spawn, dict) and "x" in spawn and "y" in spawn:
            self.map_x = float(spawn["x"])
            self.map_y = float(spawn["y"])
            self._update_world_position()
            print(
                f"    WorldStartPoint map=({self.map_x}, {self.map_y}) "
                f"world=({self.pos_x:.2f}, {self.pos_y:.2f})"
            )
        else:
            print(
                f"    WorldStartPoint not found, using "
                f"map=({self.map_x}, {self.map_y}) world=({self.pos_x:.2f}, {self.pos_y:.2f})"
            )

        size = world_bson.get("WorldSizeSettingsType", {})
        self.world_width = int(size.get("WorldSizeX", 0) or 0)
        self.world_height = int(size.get("WorldSizeY", 0) or 0)
        print(f"    World size: {self.world_width or '?'} x {self.world_height or '?'}")

        block_layer = world_bson.get("BlockLayer", b"")
        if isinstance(block_layer, (bytes, bytearray)) and len(block_layer) % 2 == 0:
            tile_ids = struct.unpack("<" + ("H" * (len(block_layer) // 2)), block_layer)
            expected_tiles = self.world_width * self.world_height
            if expected_tiles and len(tile_ids) == expected_tiles:
                self.world_tiles = tile_ids
            else:
                self.world_tiles = ()
            self.world_tile_counts = Counter(tile_ids)
            print(f"    Unique block-layer tile IDs: {len(self.world_tile_counts)}")
        else:
            self.world_tiles = ()
            self.world_tile_counts.clear()
            print("    BlockLayer missing or malformed; tile counts unavailable.")

        collectables = []
        raw_collectables = world_bson.get("Collectables", {})
        if isinstance(raw_collectables, dict):
            for key, value in raw_collectables.items():
                if key == "Count" or not isinstance(value, dict):
                    continue
                collectables.append(value)
        self.world_collectables = collectables

        raw_world_items = world_bson.get("WorldItems", {})
        if isinstance(raw_world_items, dict):
            self.world_items = dict(raw_world_items)
        else:
            self.world_items = {}

        print(
            f"    Objects: collectables={len(self.world_collectables)} "
            f"world_items={len(self.world_items)}"
        )

        if self.current_world:
            self._queue_spawn_location_sync(self.current_world)
            self._pending_spawn_setup = True

    def _cmd_loop(self):
        print(
            "Commands: join <WORLD> | leave | ready | right/left/up/down [n] | walkto <x> <y> | hit <left|up|down|right> | place <x> <y> <block_id> | place offset <dx> <dy> <block_id> | fish <left|right> <bait> | fish stop | wear <item> | unwear <item> | say <text> | inventory | players | tiles | objects | grw | pos | status | quit\n"
        )
        while self.running:
            try:
                line = input("> ").strip()
            except (EOFError, KeyboardInterrupt):
                self.running = False
                break

            if not line:
                continue

            parts = line.split()
            cmd = parts[0].lower()
            arg = parts[1] if len(parts) > 1 else ""
            rest = line[len(parts[0]) :].strip()
            try:
                n = int(arg)
            except (ValueError, IndexError):
                n = 1

            if cmd == "join" and arg:
                self.join_world(arg)
            elif cmd == "leave":
                self._queue_leave_world()
            elif cmd == "ready":
                self._request_ready()
            elif cmd in ("say", "talk"):
                self._queue_world_chat(rest)
            elif cmd in ("inventory", "inv", "bag"):
                entries = self._iter_inventory_entries()
                if not entries:
                    print("    No decoded inventory entries found yet.")
                else:
                    print(f"    Inventory entries: {len(entries)}")
                    for inventory_key, block_id, inventory_type, amount, item_name in entries:
                        print(
                            f"      key=0x{inventory_key:08X} block={block_id:<6} "
                            f"type={inventory_type:<4} amount={amount:<5} name={item_name}"
                        )
            elif cmd == "place":
                if len(parts) == 4:
                    try:
                        target_x = int(parts[1])
                        target_y = int(parts[2])
                        block_id = int(parts[3])
                    except ValueError:
                        print("    Usage: place <x> <y> <block_id>")
                        print("    Usage: place offset <dx> <dy> <block_id>")
                    else:
                        self._queue_place_block(target_x, target_y, block_id)
                elif len(parts) == 5 and parts[1].lower() == "offset":
                    try:
                        dx = int(parts[2])
                        dy = int(parts[3])
                        block_id = int(parts[4])
                    except ValueError:
                        print("    Usage: place offset <dx> <dy> <block_id>")
                    else:
                        base_x = int(round(self.map_x))
                        base_y = int(round(self.map_y))
                        self._queue_place_block(base_x + dx, base_y + dy, block_id)
                else:
                    print("    Usage: place <x> <y> <block_id>")
                    print("    Usage: place offset <dx> <dy> <block_id>")
            elif cmd == "hit":
                if arg in ("left", "up", "down", "right"):
                    self._queue_hit(arg)
                else:
                    print("    Usage: hit <left|up|down|right>")
            elif cmd == "fish":
                if len(parts) == 2 and parts[1].lower() == "stop":
                    self._stop_fishing_loop("stopped by user")
                elif len(parts) < 3:
                    print("    Usage: fish <left|right> <bait>")
                else:
                    self._queue_fish_cast(parts[1].lower(), " ".join(parts[2:]))
            elif cmd == "wear":
                if len(parts) < 2:
                    print("    Usage: wear <item>")
                else:
                    self._queue_wearable_change("wear", " ".join(parts[1:]))
            elif cmd == "unwear":
                if len(parts) < 2:
                    print("    Usage: unwear <item>")
                else:
                    self._queue_wearable_change("unwear", " ".join(parts[1:]))
            elif cmd == "grw":
                self._queue_message({"ID": "GRW"})
            elif cmd in ("players", "plist"):
                if not self.players:
                    print("    No tracked players yet.")
                else:
                    print(f"    Players in cache: {len(self.players)}")
                    for uid, player in sorted(
                        self.players.items(),
                        key=lambda item: (item[1].get("name") or "", item[0]),
                    ):
                        name = player.get("name", "?")
                        x = player.get("x", "?")
                        y = player.get("y", "?")
                        a = player.get("a", "?")
                        d = player.get("d", "?")
                        skin = player.get("skin", "?")
                        print(f"      {name:<20} uid={uid} pos=({x}, {y}) a={a} d={d} skin={skin}")
            elif cmd in ("tiles", "tilecounts", "blocks"):
                if not self.world_tile_counts:
                    print("    No tile counts loaded yet. Join a world first.")
                else:
                    print(f"    Unique tile IDs: {len(self.world_tile_counts)}")
                    for tile_id, count in sorted(self.world_tile_counts.items()):
                        tile_name = self.block_type_names.get(tile_id, "<unknown>")
                        print(f"      tile={tile_id:<6} name={tile_name:<30} count={count}")
            elif cmd in ("objects", "obj", "collectables", "worlditems"):
                if not self.world_collectables and not self.world_items:
                    print("    No world objects loaded yet. Join a world first.")
                else:
                    print(
                        f"    World objects: collectables={len(self.world_collectables)} "
                        f"world_items={len(self.world_items)}"
                    )
                    if self.world_collectables:
                        print("    Collectables:")
                        for entry in sorted(
                            self.world_collectables,
                            key=lambda item: item.get("CollectableID", 0),
                        ):
                            collectable_id = entry.get("CollectableID", "?")
                            block_type = entry.get("BlockType", "?")
                            block_name = self.block_type_names.get(block_type, "<unknown>")
                            amount = entry.get("Amount", "?")
                            inv_type = entry.get("InventoryType", "?")
                            pos_x = entry.get("PosX", "?")
                            pos_y = entry.get("PosY", "?")
                            is_gem = entry.get("IsGem", False)
                            gem_type = entry.get("GemType", "?")
                            print(
                                f"      cid={collectable_id:<6} block={block_type:<6} "
                                f"name={block_name:<30} amt={amount:<4} inv={inv_type:<3} "
                                f"pos=({pos_x}, {pos_y}) gem={is_gem} gemType={gem_type}"
                            )
                    if self.world_items:
                        print("    WorldItems:")
                        for key, value in sorted(self.world_items.items()):
                            if not isinstance(value, dict):
                                print(f"      {key}: {value}")
                                continue
                            block_type = value.get("blockType", "?")
                            block_name = self.block_type_names.get(block_type, "<unknown>")
                            class_name = value.get("class", "?")
                            item_id = value.get("itemId", "?")
                            direction = value.get("direction", "?")
                            print(
                                f"      {key:<12} itemId={item_id:<6} block={block_type:<6} "
                                f"name={block_name:<30} class={class_name:<28} dir={direction}"
                            )
            elif cmd in ("right", "left", "up", "down"):
                dx = {"right": 1.0, "left": -1.0}.get(cmd, 0.0)
                dy = {"up": 1.0, "down": -1.0}.get(cmd, 0.0)
                for _ in range(n):
                    self._move_pending.append((dx, dy))
            elif cmd in ("walkto", "path", "goto"):
                if len(parts) != 3:
                    print("    Usage: walkto <x> <y>")
                else:
                    try:
                        target_x = int(parts[1])
                        target_y = int(parts[2])
                    except ValueError:
                        print("    Usage: walkto <x> <y>")
                    else:
                        self._queue_walk_to(target_x, target_y)
            elif cmd == "pos":
                print(
                    f"    map=({self.map_x:.2f}, {self.map_y:.2f}) "
                    f"world=({self.pos_x:.2f}, {self.pos_y:.2f}) "
                    f"dir={'right' if self.move_dir == DIR_RIGHT else 'left'}"
                )
            elif cmd == "status":
                print(
                    "    "
                    f"world={self.current_world} "
                    f"in_world={self._in_world} "
                    f"awaiting_ready={self._awaiting_ready} "
                    f"spawn_report_sent={self._spawn_report_sent} "
                    f"outbox={len(self._outbox)} "
                    f"move_queue={len(self._move_pending)} "
                    f"ST={self._st_count} "
                    f"world_grid={self.world_width}x{self.world_height} "
                    f"fishing_active={self._fishing['active']} "
                    f"fishing_loop={self._fishing_loop['enabled']} "
                    f"awaiting_hook={self._fishing['awaiting_hook']} "
                    f"gauge={self._fishing['gauge_detected']} "
                    f"progress={self._fishing['sim_progress']:.2f} "
                    f"overlap={self._fishing['sim_overlap']} "
                    f"land_sent={self._fishing['land_sent']}"
                )
            elif cmd in ("quit", "exit", "q"):
                self.running = False
            else:
                print(f"    Unknown: {cmd}")

    def _service_scheduler(self, now: float):
        if self._redirect_pending:
            self._perform_pending_redirect()
            return

        if self._pending_spawn_setup and not self._outbox and self.current_world:
            self._pending_spawn_setup = False
            self._queue_world_spawn_setup(self.current_world)

        if self._spawn_packet_due and not self._outbox:
            print("[>] Spawn packet -> mp + mP(tp=true)")
            self._spawn_packet_due = False
            self._queue_spawn_packet()
            self._in_world = True

        if now >= self._next_st_at:
            self._queue_st()
            self._next_st_at = now + ST_INTERVAL
            self._st_count += 1

        if now >= self._next_keepalive_at:
            self._queue_keepalive()
            self._next_keepalive_at = now + MENU_KEEPALIVE_INTERVAL

        if self._in_world:
            if self._fishing["active"]:
                self._service_fishing(now)
            else:
                self._maybe_start_next_fishing_attempt()

            if self._move_pending and now >= self._next_move_step_at:
                dx, dy = self._move_pending.popleft()
                self._apply_move(dx, dy)
                self._next_move_step_at = now + WALK_STEP_INTERVAL
                print(f"[>] Move ({dx:+.0f},{dy:+.0f}) -> ({self.pos_x:.1f},{self.pos_y:.1f})")

            if self._movement_dirty:
                if self._map_point_dirty:
                    self._queue_map_point_update()
                    self._map_point_dirty = False
                self._queue_full_mp()
                self._movement_dirty = False
                self._next_walk_idle_at = now + WALK_IDLE_SETTLE_INTERVAL
            elif self._walk_idle_due and now >= self._next_walk_idle_at:
                self.move_anim = ANIM_IDLE
                self._queue_full_mp()
                self._walk_idle_due = False
            elif self._punch_idle_due and now >= self._next_punch_idle_at:
                self.move_anim = ANIM_IDLE
                self._queue_full_mp()
                self._punch_idle_due = False
    def _service_fishing(self, now: float):
        if not self._fishing["gauge_detected"]:
            return

        last_at = self._fishing["sim_last_at"] or now
        dt = max(0.0, min(now - last_at, 0.25))
        self._fishing["sim_last_at"] = now
        if dt <= 0.0:
            return

        phase = self._fishing["sim_phase"] + dt
        self._fishing["sim_phase"] = phase

        prev_fish = self._fishing["sim_fish_position"]
        prev_target = self._fishing["sim_target_position"]

        move_speed = self._fishing["sim_fish_move_speed"]
        run_frequency = self._fishing["sim_run_frequency"]
        base_wave = 0.18 + (move_speed * 0.05)
        burst_wave = 0.08 + (run_frequency * 1.1)

        if (
            not self._fishing["sim_run_active"]
            and now - self._fishing["gauge_entered_at"] >= 5.0
            and math.sin(phase * (0.75 + move_speed)) > (0.985 - run_frequency)
        ):
            self._fishing["sim_run_active"] = True
            self._fishing["sim_run_until"] = now + 0.45

        run_boost = 0.22 if self._fishing["sim_run_active"] else 0.0
        center = 0.5 + (base_wave + run_boost) * math.sin(phase * (0.9 + move_speed * 0.55))
        burst = burst_wave * math.sin(phase * (2.3 + move_speed * 1.1))
        fish = center + burst
        fish = max(0.0, min(1.0, fish))
        if self._fishing["sim_run_active"] and now >= self._fishing["sim_run_until"]:
            self._fishing["sim_run_active"] = False

        distance = abs(fish - prev_target)
        should_overlap = distance <= (self._fishing["sim_overlap_threshold"] * 1.35)

        force_finish = now >= self._fishing["sim_force_land_after"] > 0.0

        if force_finish:
            target = fish
        elif should_overlap:
            target = prev_target
            step = self._fishing["sim_target_speed"] * dt
            if fish > target:
                target = min(fish, target + step)
            else:
                target = max(fish, target - step)
        else:
            # When outside overlap, still move toward the fish but more slowly.
            target = prev_target
            step = (self._fishing["sim_target_speed"] * 0.35) * dt
            if fish > target:
                target = min(fish, target + step)
            else:
                target = max(fish, target - step)
        target = max(0.0, min(1.0, target))

        self._fishing["sim_fish_position"] = fish
        self._fishing["sim_target_position"] = target
        self._fishing["sim_difficulty_meter"] += abs(target - prev_target)
        self._fishing["sim_size_multiplier"] += abs(fish - prev_fish)
        self._fishing["sim_drag_extra"] = fish + 0.5

        off_distance = abs(fish - target)
        is_overlapping = off_distance <= self._fishing["sim_overlap_threshold"]
        self._fishing["sim_off_distance"] = off_distance

        prev_overlap = self._fishing["sim_overlap"]
        if is_overlapping != prev_overlap:
            self._fishing["sim_overlap"] = is_overlapping
            self._fishing["gauge_overlap_on"] = is_overlapping
            if is_overlapping:
                self._queue_message({"ID": "FiOnAM"})
            else:
                self._queue_message({"ID": "FiOffAM", "FiD": float(off_distance)})

        progress = self._fishing["sim_progress"]
        if force_finish:
            progress = max(progress, 0.985)
            progress += max(self._fishing["sim_fill_rate"] * 2.5, 0.22) * dt
        elif is_overlapping:
            progress += self._fishing["sim_fill_rate"] * dt
        else:
            drain_rate = (((off_distance * 2.5) + 0.5) * self._fishing["sim_pull_strength"]) * 0.05
            progress -= drain_rate * dt
        progress = max(0.0, min(1.0, progress))
        self._fishing["sim_progress"] = progress

        if (
            progress >= 0.999
            and is_overlapping
            and now - self._fishing["gauge_entered_at"] >= self._fishing["sim_min_land_delay"]
        ):
            if not self._fishing["sim_ready_to_land"]:
                self._fishing["sim_ready_to_land"] = True
                self._fishing["sim_ready_since"] = now
            elif not self._fishing["land_sent"] and now - self._fishing["sim_ready_since"] >= 0.15:
                self._queue_fishing_land(include_metrics=True)
        else:
            self._fishing["sim_ready_to_land"] = False
            self._fishing["sim_ready_since"] = 0.0

    def run_loop(self):
        self.running = True

        self._recv_thread = threading.Thread(target=self._recv_loop, daemon=True)
        self._recv_thread.start()

        self._cmd_thread = threading.Thread(target=self._cmd_loop, daemon=True)
        self._cmd_thread.start()

        print("[*] Game loop running. Type 'quit' or Ctrl+C to exit.\n")

        try:
            while self.running:
                now = time.time()
                self._service_scheduler(now)
                self._flush_outbox()
                time.sleep(FRAME_SLEEP)
        except KeyboardInterrupt:
            print("\n[*] Interrupted.")
        finally:
            self.running = False
            if self.sock:
                try:
                    self.sock.close()
                except OSError:
                    pass
            if self._recv_thread:
                self._recv_thread.join(timeout=1.0)


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="PixelWorld headless game client")
    parser.add_argument("--email", default=None)
    parser.add_argument("--password", default=None)
    parser.add_argument("--device", default=DEVICE_ID)
    parser.add_argument("--join", default=None, help="World to join on startup")
    parser.add_argument(
        "--manual-ready",
        action="store_true",
        help="Wait for an explicit 'ready' command instead of sending RtP on rOP.",
    )
    args = parser.parse_args()

    if args.email and args.password:
        jwt = fetch_jwt_email(args.email, args.password)
    else:
        jwt = fetch_jwt_device(args.device)

    session = GameSession(jwt, args.device, auto_ready=not args.manual_ready)
    if session.connect_and_auth():
        if args.join:
            session.join_world(args.join)
        session.run_loop()
