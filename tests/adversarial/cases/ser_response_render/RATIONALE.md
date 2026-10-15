# ser_response_render

`response_class=MyResponse` on an async route: FastAPI constructs the response class itself
after the endpoint returns (`actual_response_class(content, **response_args)` in
`fastapi.routing.get_request_handler`), on the loop. Starlette's `Response.__init__` calls
`self.render(content)` synchronously, so a `render` override doing file I/O or sleeping blocks
the event loop. User code never calls the class or `render`; the route decorator's
`response_class` argument is the only link.

Labels are on the blocking calls inside `render` (a sync function the framework calls on the
loop). `PlainResponse.render` is pure: no diagnostic. The oracle confirms both labeled lines.
