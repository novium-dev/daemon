import aiohttp
from aiohttp import web

BACKEND = "https://localhost:4000"

async def websocket_proxy(request):
    client_ws = web.WebSocketResponse()
    await client_ws.prepare(request)

    backend_url = BACKEND + str(request.rel_url)

    # Disable certificate verification for the connection
    # to your existing local HTTPS server.
    connector = aiohttp.TCPConnector(ssl=False)

    async with aiohttp.ClientSession(connector=connector) as session:
        try:
            async with session.ws_connect(backend_url) as backend_ws:

                async def client_to_backend():
                    async for msg in client_ws:
                        if msg.type == aiohttp.WSMsgType.TEXT:
                            await backend_ws.send_str(msg.data)
                        elif msg.type == aiohttp.WSMsgType.BINARY:
                            await backend_ws.send_bytes(msg.data)
                        elif msg.type == aiohttp.WSMsgType.PING:
                            await backend_ws.ping()
                        elif msg.type == aiohttp.WSMsgType.CLOSE:
                            await backend_ws.close()
                            break

                async def backend_to_client():
                    async for msg in backend_ws:
                        if msg.type == aiohttp.WSMsgType.TEXT:
                            await client_ws.send_str(msg.data)
                        elif msg.type == aiohttp.WSMsgType.BINARY:
                            await client_ws.send_bytes(msg.data)
                        elif msg.type == aiohttp.WSMsgType.PING:
                            await client_ws.ping()
                        elif msg.type == aiohttp.WSMsgType.CLOSE:
                            await client_ws.close()
                            break

                await asyncio.gather(
                    client_to_backend(),
                    backend_to_client(),
                )

        except Exception as e:
            print("WebSocket proxy error:", e)

    return client_ws


async def http_proxy(request):
    backend_url = BACKEND + str(request.rel_url)

    connector = aiohttp.TCPConnector(ssl=False)

    async with aiohttp.ClientSession(connector=connector) as session:
        body = await request.read()

        headers = dict(request.headers)
        headers.pop("Host", None)

        async with session.request(
            request.method,
            backend_url,
            headers=headers,
            data=body,
        ) as response:
            return web.Response(
                status=response.status,
                headers=response.headers,
                body=await response.read(),
            )


async def handler(request):
    if request.headers.get("Upgrade", "").lower() == "websocket":
        return await websocket_proxy(request)

    return await http_proxy(request)


app = web.Application()
app.router.add_route("*", "/{path:.*}", handler)

web.run_app(app, host="127.0.0.1", port=4001)
