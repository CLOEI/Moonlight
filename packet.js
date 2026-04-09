// Android/libil2cpp packet sniffer for Pixel World.
//
// RE basis:
// - OutgoingMessages__SerializeQueuedMessagesToBsonBytes @ 0x2254D6C
//   Returns a System.Byte[] containing the raw BSON payload.
// - AsynchronousClient__DequeueReceivedPacket @ 0x2294C64
//   Returns a System.Byte[] containing one fully reassembled inbound BSON payload.
//

const TARGET_MODULE = "libil2cpp.so";

const SEND_BSON_RVA = 0x2254D6C;
const RECV_PACKET_RVA = 0x2294C64;

// Current target is 64-bit IL2CPP. System.Byte[] layout:
//   +0x18 = array length (int32)
//   +0x20 = first byte of the data buffer
const IL2CPP_ARRAY_LENGTH_OFFSET = 0x18;
const IL2CPP_ARRAY_DATA_OFFSET = 0x20;

const MAX_PACKET_SIZE = 1024 * 1024;

function readIl2CppByteArray(byteArrayPtr) {
    if (byteArrayPtr.isNull()) {
        return null;
    }

    const len = byteArrayPtr.add(IL2CPP_ARRAY_LENGTH_OFFSET).readS32();
    if (len <= 0 || len > MAX_PACKET_SIZE) {
        return null;
    }

    const dataPtr = byteArrayPtr.add(IL2CPP_ARRAY_DATA_OFFSET);
    const data = dataPtr.readByteArray(len);
    if (data === null) {
        return null;
    }

    return { len, data };
}

// Packet log format (appended per packet):
//   [1 byte]  direction: 0x53 ('S') = send, 0x52 ('R') = recv
//   [4 bytes] timestamp low 32 bits (ms since epoch, little-endian)
//   [4 bytes] packet length (little-endian)
//   [N bytes] packet data
function writePacketToLog(direction, data, len) {
    try {
        const ts = Date.now();
        const header = new ArrayBuffer(9);
        const view = new DataView(header);
        view.setUint8(0, direction === "send" ? 0x53 : 0x52);
        view.setUint32(1, ts & 0xFFFFFFFF, true);
        view.setUint32(5, len, true);

        const f = new File("/data/local/tmp/packets.bin", "ab");
        f.write(header);
        f.write(data);
        f.close();
    } catch (e) {
        console.log("[!] Failed to write packet log: " + e);
    }
}

function isGWPacket(data) {
    if (data.byteLength < 22) return false;
    const v = new Uint8Array(data);
    return v[4]  === 0x03 &&
           v[5]  === 0x6d && v[6]  === 0x30 && v[7]  === 0x00 &&
           v[12] === 0x02 &&
           v[13] === 0x49 && v[14] === 0x44 && v[15] === 0x00 &&
           v[20] === 0x47 && v[21] === 0x57;
}

function logPacket(direction, payload) {
    console.log(
        "\n[" + (direction === "send" ? ">>>> SEND" : "<<<< RECEIVE") + "] " +
        new Date().toISOString() + " | " + payload.len + " bytes"
    );
    console.log(hexdump(payload.data, { header: true, ansi: true }));

    writePacketToLog(direction, payload.data, payload.len);

    if (direction === "recv" && isGWPacket(payload.data)) {
        try {
            const f = new File("/data/local/tmp/world.bin", "wb");
            f.write(payload.data);
            f.close();
            console.log("[*] Saved GW world data to /data/local/tmp/world.bin (" + payload.len + " bytes)");
        } catch (e) {
            console.log("[!] Failed to save GW world data: " + e);
        }
    }
}

function attachHooks(moduleBase) {
    const sendAddr = moduleBase.add(SEND_BSON_RVA);
    const recvAddr = moduleBase.add(RECV_PACKET_RVA);

    console.log("[+] " + TARGET_MODULE + " base: " + moduleBase);
    console.log("[+] Hooking send BSON builder at " + sendAddr);
    console.log("[+] Hooking recv packet dequeue at " + recvAddr);

    Interceptor.attach(sendAddr, {
        onLeave(retval) {
            try {
                const payload = readIl2CppByteArray(retval);
                if (payload !== null) {
                    logPacket("send", payload);
                }
            } catch (e) {
                console.log("[!] Send hook error: " + e);
            }
        }
    });

    Interceptor.attach(recvAddr, {
        onLeave(retval) {
            try {
                const payload = readIl2CppByteArray(retval);
                if (payload !== null) {
                    logPacket("recv", payload);
                }
            } catch (e) {
                console.log("[!] Receive hook error: " + e);
            }
        }
    });
}

function start() {
    const module = Process.findModuleByName(TARGET_MODULE);
    if (module === null) {
        console.log("[-] " + TARGET_MODULE + " not loaded yet, retrying...");
        setTimeout(start, 1000);
        return;
    }

    attachHooks(module.base);
    console.log("[+] Hooks ready. Logging packets to /data/local/tmp/packets.bin");
}

if (typeof Interceptor !== "undefined") {
    start();
} else {
    console.log("Error: Script not running inside Frida.");
}
