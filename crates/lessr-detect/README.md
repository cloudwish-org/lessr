# lessr-detect

Stage detect, proxy path, report only. Fingerprints the cacheable prefix per turn; on change, locates the first differing byte and classifies the cause (timestamp, random id, tool order, message reorder, unknown). Counting: cache_creation tokens on broken turns at write price, exact. Never mutates a request in this repo.

See ../../docs/ARCHITECTURE.md and ../../docs/MECHANISMS.md for the contracts this crate implements.
