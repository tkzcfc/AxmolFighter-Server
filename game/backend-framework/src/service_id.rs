// ═══════════════════════════════════════════════════════════════
// ServiceId — 后端服务类型标识
// ═══════════════════════════════════════════════════════════════
//
//
// 已分配的 service_id:
//   GAME    = 0  (Rust game 服)
//   BATTLE  = 1  (C++ battle 服,见 server/battle)
//   TOWN    = 2  (Rust town 服)
//

pub const SERVICE_ID_GAME: u32 = 0;
pub const SERVICE_ID_BATTLE: u32 = 1;
pub const SERVICE_ID_TOWN: u32 = 2;
