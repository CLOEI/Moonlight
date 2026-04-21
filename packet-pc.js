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
        view.setUint8(0, direction === 'send' ? 0x53 : 0x52);
        view.setUint32(1, ts & 0xFFFFFFFF, true);
        view.setUint32(5, len, true);

        const f = new File("packets.bin", "ab");
        f.write(header);
        f.write(data);
        f.close();
    } catch (e) {
        console.log("[!] Failed to write packet log: " + e);
    }
}

function start() {
    // Process.getModuleByName will throw an error if not found,
    // findModuleByName returns null.
    const module = Process.findModuleByName("GameAssembly.dll");

    if (module !== null) {
        console.log("[+] Found GameAssembly.dll at: " + module.base);

        // dump.cs (2026-04-20)
        // - OutgoingMessages.TurnMessagesToBytesAndConsumeThem()  RVA 0x970CF0
        // - AsynchronousClient.GetAndConsumeFirstPacketForClient() RVA 0x933470
        const sendRVA = 0x970CF0;
        const receiveRVA = 0x933470;

        // Hook SEND
        Interceptor.attach(module.base.add(sendRVA), {
            onLeave: function(retval) {
                if (retval.isNull()) return;
                try {
                    // retval + 0x18 is length, + 0x20 is data
                    const len = retval.add(0x18).readInt();
                    if (len > 0 && len < 50000) {
                        const data = retval.add(0x20).readByteArray(len);
                        console.log("\n[>>>> SEND] " + new Date().toISOString() + " | " + len + " bytes");
                        console.log(hexdump(data, { header: true, ansi: true }));
                        writePacketToLog('send', data, len);
                    }
                } catch (e) {}
            }
        });

        // Hook RECEIVE
        Interceptor.attach(module.base.add(receiveRVA), {
            onLeave: function(retval) {
                if (retval.isNull()) return;
                try {
                    const len = retval.add(0x18).readInt();
                    if (len > 0 && len < 50000) {
                        const data = retval.add(0x20).readByteArray(len);
                        console.log("\n[<<<< RECEIVE] " + new Date().toISOString() + " | " + len + " bytes");
                        console.log(hexdump(data, { header: true, ansi: true }));
                        writePacketToLog('recv', data, len);

                        if (isGWPacket(data)) {
                            const f = new File("world.bin", "wb");
                            f.write(data);
                            f.close();
                            console.log("[*] Saved GW world data to world.bin (" + len + " bytes)");
                        }
                    }
                } catch (e) {}
            }
        });

        console.log("[+] All hooks ready. Logging packets to packets.bin");
    } else {
        console.log("[-] DLL not loaded yet, retrying...");
        setTimeout(start, 1000);
    }
}

function isGWPacket(data) {
    if (data.byteLength < 22) return false;
    const v = new Uint8Array(data);
    // BSON layout (from observed GW packet):
    //   [0-3]  outer doc size
    //   [4]    0x03  embedded doc type
    //   [5-7]  "m0\0"
    //   [8-11] inner doc size
    //   [12]   0x02  string type
    //   [13-15] "ID\0"
    //   [16-19] string length
    //   [20-21] "GW"  <-- ID value prefix
    return v[4]  === 0x03 &&
           v[5]  === 0x6d && v[6]  === 0x30 && v[7]  === 0x00 &&
           v[12] === 0x02 &&
           v[13] === 0x49 && v[14] === 0x44 && v[15] === 0x00 &&
           v[20] === 0x47 && v[21] === 0x57;
}

// Global check to ensure we are in a Frida context
if (typeof Interceptor !== 'undefined') {
    start();
} else {
    console.log("Error: Script not running inside Frida.");
}