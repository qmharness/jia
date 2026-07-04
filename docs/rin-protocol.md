// ── Rin Unix Socket Listener ─────────────────────────────
//
// Listens on ~/.jia/rin.sock for jia-rin (the ambient macOS agent)
// and jia-tui (the terminal UI). Uses the same JSON-line protocol as
// the SSE stream — each line is a complete JSON object with a "type" field.
//
// Protocol:
//   client → jia:
//     {"type":"agent","messages":[...],"session_id":"..."}
//     {"type":"cancel","session_id":"..."}
//     {"type":"confirm","id":"...","token":"...","approved":true}
//     {"type":"answer","id":"...","token":"...","answer":"..."}
//     {"type":"sessions"}
//     {"type":"load_session","session_id":"..."}
//
//   jia → client:
//     {"type":"delta","content":"..."}
//     {"type":"session","session_id":"..."}
//     {"type":"done"}
//     {"type":"cron_notification","job_name":"...","response":"...","timestamp":...}
//     {"type":"sessions","sessions":[...]}
//     {"type":"session_history","session_id":"...","entries":[...]}
//     {"type":"confirm_resolved","id":"...","resolved":true}
//     {"type":"answer_resolved","id":"...","resolved":true}

