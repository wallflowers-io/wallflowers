"""The macOS Security.framework binding — ctypes, no third-party dependency.

Why ctypes and not ``keyring``: this package needs the **raw OSStatus**.
``keyring`` (and ``security(1)``, and every convenience wrapper) collapses
"no such item" and "the keychain refused to answer" into one falsy return,
which is exactly the conflation that DeviceKey.swift's header documents as a
data-loss bug. A wrapper that cannot tell those apart cannot implement the
contract, so this module talks to ``SecItemCopyMatching`` directly.

It is also why there is no dependency to install: the workspace's Python is
provisioned and not ``pip install``-ed into (the doctrine, working notes),
and stdlib ``ctypes`` is enough.

Nothing here is macOS-specific by preference — it is macOS-specific because
that is the secure store that exists on this platform. See ``keychain.py`` for
the platform gate.
"""
from __future__ import annotations

import ctypes
import sys
from ctypes import byref, c_int32, c_long, c_void_p

#: OSStatus values we branch on. From <Security/SecBase.h>.
errSecSuccess = 0
errSecDuplicateItem = -25299
errSecItemNotFound = -25300
errSecInteractionNotAllowed = -25308
errSecDecode = -26275

_CF_PATH = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation"
_SEC_PATH = "/System/Library/Frameworks/Security.framework/Security"

#: kCFStringEncodingUTF8
_UTF8 = 0x08000100


class _Framework:
    """The two dylibs and the constants, loaded once and lazily.

    Lazy because importing ``wallflowers`` on Linux must not explode — the env
    var path (``atrest.resolve``) works anywhere, and only the keychain calls
    need Darwin.
    """

    def __init__(self) -> None:
        self.cf = ctypes.CDLL(_CF_PATH)
        self.sec = ctypes.CDLL(_SEC_PATH)
        cf, sec = self.cf, self.sec

        cf.CFStringCreateWithCString.restype = c_void_p
        cf.CFStringCreateWithCString.argtypes = [c_void_p, ctypes.c_char_p, ctypes.c_uint32]
        cf.CFDataCreate.restype = c_void_p
        cf.CFDataCreate.argtypes = [c_void_p, ctypes.c_char_p, c_long]
        cf.CFDictionaryCreate.restype = c_void_p
        cf.CFDictionaryCreate.argtypes = [c_void_p, c_void_p, c_void_p, c_long, c_void_p, c_void_p]
        cf.CFRelease.argtypes = [c_void_p]
        cf.CFDataGetBytePtr.restype = c_void_p
        cf.CFDataGetBytePtr.argtypes = [c_void_p]
        cf.CFDataGetLength.restype = c_long
        cf.CFDataGetLength.argtypes = [c_void_p]
        cf.CFGetTypeID.restype = c_long
        cf.CFGetTypeID.argtypes = [c_void_p]
        cf.CFDataGetTypeID.restype = c_long

        sec.SecItemCopyMatching.restype = c_int32
        sec.SecItemCopyMatching.argtypes = [c_void_p, c_void_p]
        sec.SecItemAdd.restype = c_int32
        sec.SecItemAdd.argtypes = [c_void_p, c_void_p]
        sec.SecItemUpdate.restype = c_int32
        sec.SecItemUpdate.argtypes = [c_void_p, c_void_p]
        sec.SecItemDelete.restype = c_int32
        sec.SecItemDelete.argtypes = [c_void_p]

        self.kSecClass = self._sec_const("kSecClass")
        self.kSecClassGenericPassword = self._sec_const("kSecClassGenericPassword")
        self.kSecAttrService = self._sec_const("kSecAttrService")
        self.kSecAttrAccount = self._sec_const("kSecAttrAccount")
        self.kSecValueData = self._sec_const("kSecValueData")
        self.kSecReturnData = self._sec_const("kSecReturnData")
        self.kSecMatchLimit = self._sec_const("kSecMatchLimit")
        self.kSecMatchLimitOne = self._sec_const("kSecMatchLimitOne")
        self.kSecAttrAccessible = self._sec_const("kSecAttrAccessible")
        # Device-only, available after first unlock — the same class DeviceKey.swift
        # and Keychain.swift use. ThisDeviceOnly is load-bearing: it excludes the
        # item from backup and device transfer, which is the property that makes
        # the sealed store device-bound.
        self.kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly = self._sec_const(
            "kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly"
        )
        self.kCFBooleanTrue = c_void_p.in_dll(self.cf, "kCFBooleanTrue").value

    def _sec_const(self, name: str) -> int:
        return c_void_p.in_dll(self.sec, name).value

    # -- CoreFoundation constructors. Every object made here is owned by the
    # -- caller; `_Scope` below releases them.
    def string(self, text: str) -> int:
        return self.cf.CFStringCreateWithCString(None, text.encode("utf-8"), _UTF8)

    def data(self, raw: bytes) -> int:
        return self.cf.CFDataCreate(None, raw, len(raw))

    def dictionary(self, pairs: list[tuple[int, int]]) -> int:
        n = len(pairs)
        keys = (c_void_p * n)(*[k for k, _ in pairs])
        values = (c_void_p * n)(*[v for _, v in pairs])
        return self.cf.CFDictionaryCreate(None, keys, values, n, None, None)


_fw: _Framework | None = None


def framework() -> _Framework:
    """The loaded frameworks, or raise if this is not macOS."""
    global _fw
    if _fw is None:
        if sys.platform != "darwin":
            from .errors import Unsupported

            raise Unsupported(
                f"no keychain on {sys.platform!r}; set $PACIFIC_ATREST_KEY "
                "(64 hex chars) instead"
            )
        _fw = _Framework()
    return _fw


class Scope:
    """Releases the CoreFoundation objects made inside it.

    CFDictionaryCreate does not retain the keys and values we hand it in the
    way a Python caller expects, so every CFString/CFData/CFDictionary made for
    one call is released together when the call returns.
    """

    def __init__(self) -> None:
        self._owned: list[int] = []

    def keep(self, ref: int) -> int:
        if ref:
            self._owned.append(ref)
        return ref

    def __enter__(self) -> "Scope":
        return self

    def __exit__(self, *exc: object) -> None:
        cf = framework().cf
        for ref in reversed(self._owned):
            cf.CFRelease(ref)
        self._owned.clear()


def query_bytes(service: str, account: str) -> tuple[int, bytes | None]:
    """``SecItemCopyMatching`` for one generic password.

    Returns ``(status, data)``. The status is returned rather than raised so
    the caller can make the not-found/unreadable decision explicitly — the one
    decision this whole package is arranged around.
    """
    fw = framework()
    with Scope() as sc:
        q = sc.keep(
            fw.dictionary(
                [
                    (fw.kSecClass, fw.kSecClassGenericPassword),
                    (fw.kSecAttrService, sc.keep(fw.string(service))),
                    (fw.kSecAttrAccount, sc.keep(fw.string(account))),
                    (fw.kSecReturnData, fw.kCFBooleanTrue),
                    (fw.kSecMatchLimit, fw.kSecMatchLimitOne),
                ]
            )
        )
        out = c_void_p()
        status = fw.sec.SecItemCopyMatching(q, byref(out))
        if status != errSecSuccess:
            return status, None
        if fw.cf.CFGetTypeID(out) != fw.cf.CFDataGetTypeID():
            return errSecDecode, None
        length = fw.cf.CFDataGetLength(out)
        raw = ctypes.string_at(fw.cf.CFDataGetBytePtr(out), length)
        fw.cf.CFRelease(out)
        return errSecSuccess, raw


def add_or_update(service: str, account: str, raw: bytes) -> int:
    """Add, then update on duplicate. **Never delete-then-add.**

    A delete that succeeds followed by an add that fails would leave the device
    with no key and a sealed store it can never open again. DeviceKey.swift
    says the same thing in the same order, and for the same reason.
    """
    fw = framework()
    with Scope() as sc:
        base = [
            (fw.kSecClass, fw.kSecClassGenericPassword),
            (fw.kSecAttrService, sc.keep(fw.string(service))),
            (fw.kSecAttrAccount, sc.keep(fw.string(account))),
        ]
        add = sc.keep(
            fw.dictionary(
                base
                + [
                    (fw.kSecValueData, sc.keep(fw.data(raw))),
                    (
                        fw.kSecAttrAccessible,
                        fw.kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
                    ),
                ]
            )
        )
        status = fw.sec.SecItemAdd(add, None)
        if status == errSecSuccess:
            return status
        if status != errSecDuplicateItem:
            return status
        return fw.sec.SecItemUpdate(
            sc.keep(fw.dictionary(base)),
            sc.keep(fw.dictionary([(fw.kSecValueData, sc.keep(fw.data(raw)))])),
        )


def delete(service: str, account: str) -> int:
    """``SecItemDelete`` for one generic password."""
    fw = framework()
    with Scope() as sc:
        q = sc.keep(
            fw.dictionary(
                [
                    (fw.kSecClass, fw.kSecClassGenericPassword),
                    (fw.kSecAttrService, sc.keep(fw.string(service))),
                    (fw.kSecAttrAccount, sc.keep(fw.string(account))),
                ]
            )
        )
        return fw.sec.SecItemDelete(q)
