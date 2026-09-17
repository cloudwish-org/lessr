# lessr-receipt

Recorder: mpsc channel and a background thread batching into SQLite (WAL). Tables: sessions, turns, savings, handles. Serves lessr gain and the self-healing table (expand rate per filter per repo). Nothing on the hot path calls this crate synchronously.

See ../../docs/ARCHITECTURE.md and ../../docs/MECHANISMS.md for the contracts this crate implements.
