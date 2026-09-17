# lessr-proxy

localhost:7433. Routes /v1/messages and /v1/chat/completions, forwards with original headers, streams the response untouched, parses usage from the final SSE frame or JSON body, emits Usage to the recorder. No buffering of bodies.

See ../../docs/ARCHITECTURE.md and ../../docs/MECHANISMS.md for the contracts this crate implements.
