"""
Pixel World authentication client.

Replicates the auth handshake up to receiving player data (GPd response).

Usage:
  python auth_client.py [--email YOUR_EMAIL --password YOUR_PASSWORD]
"""

import argparse
import base64
import json
import socket
import struct
import sys
import urllib.request
import uuid
from datetime import datetime, timezone

try:
    from bson import BSON, decode_all  # noqa: F401
    import bson

    def encode_bson(doc: dict) -> bytes:
        return BSON.encode(doc)

    def decode_bson(data: bytes) -> dict:
        return bson.decode(data)

    BSON_LIB = "pymongo/bson"
except ImportError:
    try:
        import bson

        def encode_bson(doc: dict) -> bytes:
            return bson.dumps(doc)

        def decode_bson(data: bytes) -> dict:
            return bson.loads(data)

        BSON_LIB = "bson (standalone)"
    except ImportError:
        print("[!] No BSON library found. Install with: pip install pymongo")
        sys.exit(1)


SERVER_HOST = "3.79.101.106"
SERVER_PORT = 10001
RELAUNCH_PASS = "#m(y+JxiHzFNXJnOo&UHpVwOyV1R%wP"
DEVICE_ID = "57ce9585c26da4fe279588e2414f4935a6318955"


def wrap_packet(msg_dict: dict) -> bytes:
    outer = {"m0": msg_dict, "mc": 1}
    bson_data = encode_bson(outer)
    return struct.pack("<I", len(bson_data) + 4) + bson_data


def recv_packet(sock: socket.socket) -> dict:
    header = _recv_exact(sock, 4)
    total_len = struct.unpack("<I", header)[0]
    if total_len < 4:
        raise ValueError(f"Bad packet length: {total_len}")

    bson_data = _recv_exact(sock, total_len - 4)
    return decode_bson(bson_data)


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("Connection closed by server")
        buf += chunk
    return buf


def make_vchk(device_id: str = None) -> dict:
    if device_id is None:
        device_id = uuid.uuid4().hex + uuid.uuid4().hex[:8]
    return {
        "ID": "VChk",
        "OS": "WindowsPlayer",
        "OSt": 3,
        "sdid": device_id,
    }


def make_gpd(jwt_token: str, cgy: int = 0x036D, pw: str = RELAUNCH_PASS) -> dict:
    return {
        "ID": "GPd",
        "AT": jwt_token,
        "cgy": cgy,
        "Pw": pw,
    }


def pretty(obj, indent=0):
    pad = "  " * indent
    if isinstance(obj, dict):
        for k, v in obj.items():
            if isinstance(v, (dict, list)):
                print(f"{pad}{k}:")
                pretty(v, indent + 1)
            elif isinstance(v, bytes):
                print(f"{pad}{k}: <bytes len={len(v)}>")
            else:
                print(f"{pad}{k}: {v!r}")
    elif isinstance(obj, list):
        for i, item in enumerate(obj):
            print(f"{pad}[{i}]:")
            pretty(item, indent + 1)
    else:
        print(f"{pad}{obj!r}")


def extract_msg(outer: dict) -> dict:
    return outer.get("m0", outer)


def summarize_gpd_response(outer: dict):
    data = extract_msg(outer)
    msg_id = data.get("ID", "?")
    print(f"\n{'=' * 60}")
    print(f"  Response ID : {msg_id}")
    if msg_id == "GPd":
        print(f"  UserID (U)  : {data.get('U', 'N/A')}")
        print(f"  Username(UN): {data.get('UN', 'N/A')}")
        pD = data.get("pD", {})
        if isinstance(pD, bytes):
            try:
                pD = decode_bson(pD)
            except Exception as e:
                print(f"  [!] Failed to decode pD BSON: {e}")
                pD = {}
        print(f"  Gems        : {pD.get('gems', 'N/A')}")
        print(f"  XP Amount   : {pD.get('xpAmount', 'N/A')}")
        print(f"  Email       : {data.get('Email', 'N/A')}")
        print(f"  EmailVerif  : {data.get('EmailVerified', 'N/A')}")
        print(f"  Country     : {pD.get('countryCode', 'N/A')}")
        print(f"  rUN         : {data.get('rUN', 'N/A')}")
        if isinstance(pD, dict):
            print("\n  --- pD (player data) top-level keys ---")
            for k in pD.keys():
                print(f"       {k}")
    print(f"{'=' * 60}\n")


def fetch_jwt_device(device_id: str) -> str:
    print("[*] Logging into PlayFab via Android Device ID...")
    playfab_req = urllib.request.Request(
        "https://11ef5c.playfabapi.com/Client/LoginWithAndroidDeviceID",
        data=json.dumps(
            {
                "AndroidDeviceId": device_id,
                "CreateAccount": True,
                "TitleId": "11EF5C",
            }
        ).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(playfab_req) as r:
        session_ticket = json.loads(r.read().decode())["data"]["SessionTicket"]
    return _exchange_playfab_ticket(session_ticket)


def fetch_jwt_email(email: str, password: str) -> str:
    print(f"[*] Logging into PlayFab via Email: {email} ...")
    playfab_req = urllib.request.Request(
        "https://11ef5c.playfabapi.com/Client/LoginWithEmailAddress",
        data=json.dumps(
            {
                "Email": email,
                "Password": password,
                "TitleId": "11EF5C",
            }
        ).encode(),
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(playfab_req) as r:
            session_ticket = json.loads(r.read().decode())["data"]["SessionTicket"]
        return _exchange_playfab_ticket(session_ticket)
    except urllib.error.HTTPError as e:
        print(f"[!] PlayFab Login Failed: {e.code} {e.read().decode()}")
        sys.exit(1)


def _exchange_playfab_ticket(session_ticket: str) -> str:
    print("[*] Exchanging PlayFab SessionTicket for SocialFirst JWT...")
    exchange_req = urllib.request.Request(
        "https://pw-auth.pw.sclfrst.com/v1/auth/exchangeToken",
        data=json.dumps({"playfabToken": session_ticket}).encode(),
        headers={
            "Content-Type": "application/json",
            "X-Sf-Client-Api-Key": "QwvzCrL2CexvXs2798fetBjty",
            "X-Unity-Version": "6000.3.11f1",
        },
    )
    with urllib.request.urlopen(exchange_req) as r:
        return json.loads(r.read().decode())["socialFirstToken"]


def _print_jwt_summary(jwt_token: str):
    try:
        payload_b64 = jwt_token.split(".")[1]
        payload_b64 += "=" * (4 - len(payload_b64) % 4)
        jwt_payload = json.loads(base64.urlsafe_b64decode(payload_b64))
        exp_ts = jwt_payload.get("exp", 0)
        exp_dt = datetime.fromtimestamp(exp_ts, tz=timezone.utc)
        print("[*] JWT subject     :")
        print(f"  sub      : {jwt_payload.get('sub')}")
        print(f"  nickname : {jwt_payload.get('nickname')}")
        print(f"  email    : {jwt_payload.get('email')}")
        print(
            f"  exp      : {exp_dt} UTC  "
            f"(NOW={datetime.now(timezone.utc).strftime('%Y-%m-%d %H:%M:%S')} UTC)"
        )
    except Exception as e:
        print(f"[*] JWT decode skipped: {e}")
    print()


def run(device_id: str = None, email: str = None, password: str = None, verbose: bool = False):
    if not device_id:
        device_id = DEVICE_ID

    print(f"[*] Using BSON lib  : {BSON_LIB}")
    print(f"[*] Target          : {SERVER_HOST}:{SERVER_PORT}")
    print("[*] Fetching a fresh JWT...")
    if email and password:
        jwt_token = fetch_jwt_email(email, password)
    else:
        jwt_token = fetch_jwt_device(device_id)
    print("[+] Got a new JWT!")
    _print_jwt_summary(jwt_token)

    print(f"[1] Connecting to {SERVER_HOST}:{SERVER_PORT} ...")
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    sock.settimeout(10)
    sock.connect((SERVER_HOST, SERVER_PORT))
    print("    Connected.\n")

    try:
        vchk_msg = make_vchk(device_id)
        pkt = wrap_packet(vchk_msg)
        print(f"[2] SEND VChk  ({len(pkt)} bytes)")
        if verbose:
            print(f"    msg = {vchk_msg}")
        sock.sendall(pkt)

        print("[3] RECV VChk ack ...")
        outer = recv_packet(sock)
        resp = extract_msg(outer)
        resp_id = resp.get("ID", "?")
        vn = resp.get("VN", None)
        print(f"    ID={resp_id}  VN={vn}")
        if resp_id != "VChk":
            print(f"    [!] Unexpected response ID: {resp_id}. Full response:")
            pretty(outer)
            return
        if vn == 200:
            print("    Version OK (VN=200)\n")
        else:
            print(f"    [!] Unexpected VN={vn}. Proceeding anyway...\n")

        gpd_msg = make_gpd(jwt_token)
        pkt = wrap_packet(gpd_msg)
        print(f"[4] SEND GPd   ({len(pkt)} bytes)")
        if verbose:
            print(f"    AT[:40]={jwt_token[:40]}...")
        sock.sendall(pkt)

        print("[5] RECV player data ...")
        outer = recv_packet(sock)
        resp = extract_msg(outer)
        resp_id = resp.get("ID", "?")
        print(f"    ID={resp_id}  payload size ~{len(encode_bson(outer))} bytes")

        summarize_gpd_response(outer)

        if verbose:
            print("\n[*] Full decoded response:")
            pretty(outer)

        print("[*] Auth flow complete.")
    finally:
        sock.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Pixel World auth client")
    parser.add_argument(
        "--device",
        default=None,
        help="Device unique identifier (40-char hex). Random if omitted.",
    )
    parser.add_argument("--email", default=None, help="Email for PlayFab login")
    parser.add_argument("--password", default=None, help="Password for PlayFab login")
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Print full decoded BSON responses",
    )
    args = parser.parse_args()

    run(
        device_id=args.device,
        email=args.email,
        password=args.password,
        verbose=args.verbose,
    )
