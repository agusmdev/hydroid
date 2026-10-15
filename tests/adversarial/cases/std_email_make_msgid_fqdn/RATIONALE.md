# email.utils.make_msgid() does a DNS lookup

Email helpers routinely set `msg["Message-ID"] = make_msgid()`. With no `domain=` argument,
`email.utils.make_msgid` calls `socket.getfqdn()`, which performs blocking hostname resolution
(gethostbyaddr) - often slow in containers with broken reverse DNS. Called from an `async def`
endpoint, it blocks the loop.

hydroid likely misses it because `make_msgid` is a stdlib function not in the catalog, and the
blocking `socket.getfqdn` call lives inside the stdlib body.

Labels: runtime confirms `socket.getfqdn` on the loop at line 31. Negatives: `make_msgid(domain=...)`
skips getfqdn (no runtime call); the threaded variant offloads with `asyncio.to_thread`.
