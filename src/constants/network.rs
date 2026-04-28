pub const SERVER_HOST: &str = "game-lava.pixelworlds.pw"; // serverAddress AsynchronousClient
pub const SERVER_PORT: u16 = 10001;
pub const RELAUNCH_PASS: &str = "F3nal19jzMHWWzKA#GWB";
pub const PLAYFAB_TITLE_ID: &str = "11EF5C";
pub const SOCIALFIRST_API_KEY: &str = "QwvzCrL2CexvXs2798fetBjty";
pub const UNITY_VERSION: &str = "6000.3.11f1";
pub const PLAYFAB_EMAIL_URL: &str = "https://11ef5c.playfabapi.com/Client/LoginWithEmailAddress";
pub const PLAYFAB_ANDROID_URL: &str =
    "https://11ef5c.playfabapi.com/Client/LoginWithAndroidDeviceID";
pub const SOCIALFIRST_EXCHANGE_URL: &str = "https://pw-auth.pw.sclfrst.com/v1/auth/exchangeToken";
pub const DEFAULT_DEVICE_ID: &str = "57ce9585c26da4fe279588e2414f4935a6318955";
pub const DASHBOARD_BIND_ADDR: &str = "0.0.0.0:3000";

pub fn dashboard_bind_addr() -> &'static str {
    DASHBOARD_BIND_ADDR
}
