# lessr-packs

Pack format (manifest.toml + rules/*.toml), ed25519 signature verification with the shipped public key, base pack embedded via include_bytes, compiled RegexSet cache memory-mapped by pack hash. Refuses unsigned or altered packs.  subcommand regenerates a manifest.

See ../../docs/ARCHITECTURE.md and ../../docs/MECHANISMS.md for the contracts this crate implements.
