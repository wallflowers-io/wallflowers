"""The one exception that ends a request on purpose.

It used to live in oidc.py, because OIDC verification was where most of them
were raised. That module is gone with the providers it verified, and this
class outlived it: every ceremony, every gate and every refusal in the service
still says "stop, and tell the person this code" the same way.
"""
from __future__ import annotations


class AuthError(Exception):
    """Anything that should end the request. `code` is what the client is
    shown; `detail` is for the log and never crosses the wire."""

    def __init__(self, code: str, detail: str = ""):
        super().__init__(detail or code)
        self.code = code
        self.detail = detail
