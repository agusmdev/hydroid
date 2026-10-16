"""Project classes registered with a library object, which builds them and calls their methods
later from its own sync methods (authlib's `AuthorizationServer`, as Polar uses it)."""

import time
import typing

from authlib.oauth2 import AuthorizationServer as _AuthorizationServer
from authlib.oauth2.rfc6749 import ClientMixin, OAuth2Request
from authlib.oauth2.rfc6749.grants import RefreshTokenGrant as _RefreshTokenGrant
from authlib.oauth2.rfc6749.requests import BasicOAuth2Payload
from authlib.oauth2.rfc7009 import RevocationEndpoint as _RevocationEndpoint
from fastapi import FastAPI
from starlette.concurrency import run_in_threadpool
from starlette.responses import JSONResponse


class Client(ClientMixin):
    def get_client_id(self):
        return "client"

    def check_client_secret(self, client_secret):
        return True

    def check_endpoint_auth_method(self, method, endpoint):
        return True

    def check_grant_type(self, grant_type):
        return True


class RefreshTokenGrant(_RefreshTokenGrant):
    TOKEN_ENDPOINT_AUTH_METHODS = ["client_secret_post"]

    # authlib calls it from `create_token_response`, on the grant it builds from the class.
    def authenticate_refresh_token(self, refresh_token: str):
        time.sleep(0.1)


class RevocationEndpoint(_RevocationEndpoint):
    CLIENT_AUTH_METHODS = ["client_secret_post"]

    def query_token(self, token_string, token_type_hint):
        time.sleep(0.1)


class Framework:
    # What a framework integration implements.
    def query_client(self, client_id: str):
        return Client()

    def create_oauth2_request(self, request):
        return request

    def handle_response(self, status_code, payload, headers):
        return JSONResponse(payload, status_code)

    def send_signal(self, name, *args, **kwargs):
        pass


class Server(Framework, _AuthorizationServer):
    # Only for type checkers: calling it runs the library method.
    if typing.TYPE_CHECKING:

        def create_endpoint_response(self, name: str, request=None) -> JSONResponse: ...

    @classmethod
    def build(cls) -> typing.Self:
        server = cls(scopes_supported=["openid"])
        server.register_endpoint(RevocationEndpoint)
        register_grants(server)
        return server


def register_grants(server: Server) -> None:
    server.register_grant(RefreshTokenGrant)


class SlowClientServer(Server):
    # `get_authorization_grant`, a sync library method, calls `self.query_client(...)`.
    def query_client(self, client_id: str):
        time.sleep(0.1)
        return Client()

    # Never called by the library.
    def audit(self) -> None:
        time.sleep(0.1)


server = Server.build()
slow_client_server = SlowClientServer()


class QuietGrant(_RefreshTokenGrant):
    TOKEN_ENDPOINT_AUTH_METHODS = ["client_secret_post"]

    def authenticate_refresh_token(self, refresh_token: str):
        return None


# Registries are known by the class of the object keeping them, not by object: a server of
# another class.
class QuietServer(Framework, _AuthorizationServer):
    pass


quiet = QuietServer()
quiet.register_grant(QuietGrant)


class FormRequest(OAuth2Request):
    def __init__(self, form: dict[str, str]) -> None:
        super().__init__("POST", "https://example.com/oauth2")
        self.payload = BasicOAuth2Payload(form)
        self._form = form

    @property
    def form(self):
        return self._form


def oauth2_request(**form: str) -> FormRequest:
    return FormRequest({"client_id": "client", "client_secret": "secret", **form})


app = FastAPI()


@app.post("/token")
async def token(refresh_token: str):
    request = oauth2_request(grant_type="refresh_token", refresh_token=refresh_token)
    return server.create_token_response(request)  # expect: time.sleep via route POST /token


@app.post("/revoke")
async def revoke(token: str):
    request = oauth2_request(token=token)
    return server.create_endpoint_response("revocation", request)  # expect: time.sleep via route POST /revoke


@app.get("/authorize")
async def authorize():
    request = oauth2_request(response_type="code")
    return slow_client_server.create_authorization_response(request)  # expect: time.sleep via route GET /authorize


@app.get("/scopes")
async def scopes():
    # Reads no registry and calls no override.
    server.validate_requested_scope("openid")
    return {"ok": True}


@app.post("/quiet-token")
async def quiet_token(refresh_token: str):
    # The grant registered on this server does not block.
    request = oauth2_request(grant_type="refresh_token", refresh_token=refresh_token)
    return quiet.create_token_response(request)


@app.post("/offloaded-token")
async def offloaded_token(refresh_token: str):
    request = oauth2_request(grant_type="refresh_token", refresh_token=refresh_token)
    return await run_in_threadpool(server.create_token_response, request)


@app.post("/threadpool-token")
def threadpool_token(refresh_token: str):
    request = oauth2_request(grant_type="refresh_token", refresh_token=refresh_token)
    return server.create_token_response(request)
