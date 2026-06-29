// Minimal WebAuthn browser plumbing (no npm dependency). Converts the base64url
// fields in webauthn-rs's challenge JSON to ArrayBuffers for navigator.credentials,
// and serialises the authenticator's response back to the base64url JSON the hub
// expects. Mirrors what @simplewebauthn/browser does, kept tiny + inline.

function b64urlToBuf(s) {
  const pad = '='.repeat((4 - (s.length % 4)) % 4)
  const b64 = (s + pad).replace(/-/g, '+').replace(/_/g, '/')
  const bin = atob(b64)
  const buf = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i)
  return buf.buffer
}

function bufToB64url(buf) {
  const bytes = new Uint8Array(buf)
  let bin = ''
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i])
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

export function supported() {
  return typeof window !== 'undefined' && !!window.PublicKeyCredential && !!navigator.credentials
}

// ---- registration ----

// `options` = the JSON from POST /register/start ({ publicKey: {...} }).
export async function register(options) {
  const pk = { ...options.publicKey }
  pk.challenge = b64urlToBuf(pk.challenge)
  pk.user = { ...pk.user, id: b64urlToBuf(pk.user.id) }
  if (pk.excludeCredentials) pk.excludeCredentials = pk.excludeCredentials.map((c) => ({ ...c, id: b64urlToBuf(c.id) }))
  const cred = await navigator.credentials.create({ publicKey: pk })
  return {
    id: cred.id,
    rawId: bufToB64url(cred.rawId),
    type: cred.type,
    response: {
      attestationObject: bufToB64url(cred.response.attestationObject),
      clientDataJSON: bufToB64url(cred.response.clientDataJSON),
    },
    extensions: cred.getClientExtensionResults ? cred.getClientExtensionResults() : {},
  }
}

// ---- authentication ----

// `options` = the JSON from the login response's `passkey` field ({ publicKey: {...} }).
export async function authenticate(options) {
  const pk = { ...options.publicKey }
  pk.challenge = b64urlToBuf(pk.challenge)
  if (pk.allowCredentials) pk.allowCredentials = pk.allowCredentials.map((c) => ({ ...c, id: b64urlToBuf(c.id) }))
  const cred = await navigator.credentials.get({ publicKey: pk })
  return {
    id: cred.id,
    rawId: bufToB64url(cred.rawId),
    type: cred.type,
    response: {
      authenticatorData: bufToB64url(cred.response.authenticatorData),
      clientDataJSON: bufToB64url(cred.response.clientDataJSON),
      signature: bufToB64url(cred.response.signature),
      userHandle: cred.response.userHandle ? bufToB64url(cred.response.userHandle) : null,
    },
    extensions: cred.getClientExtensionResults ? cred.getClientExtensionResults() : {},
  }
}
