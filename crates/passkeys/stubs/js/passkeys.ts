// Published by `art publish passkeys --with-ui`. App code: change it freely.
//
// The browser half of each passkey ceremony: fetch options from the server,
// ask the browser (`navigator.credentials`), post the answer back through
// Inertia so redirects and validation errors work like any form.
import { router } from '@inertiajs/vue3'

/** POST with the CSRF header the `csrf` layer checks (from the XSRF-TOKEN cookie). */
async function options(url: string): Promise<any> {
  const xsrf = document.cookie.match(/(?:^|; )XSRF-TOKEN=([^;]*)/)?.[1] ?? ''
  const res = await fetch(url, {
    method: 'POST',
    headers: { Accept: 'application/json', 'X-XSRF-TOKEN': decodeURIComponent(xsrf) },
    credentials: 'same-origin',
  })
  if (!res.ok) throw new Error(`passkey options: ${res.status}`)
  return res.json()
}

async function get(url: string): Promise<PublicKeyCredential | null> {
  const { publicKey } = await options(url)
  // Modal prompt; the options' `mediation: "conditional"` is for autofill only.
  return (await navigator.credentials.get({
    publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(publicKey),
  })) as PublicKeyCredential | null
}

const cancelled = (e: unknown) => e instanceof DOMException && e.name === 'NotAllowedError'

/** Add a passkey to the signed-in account. */
export async function register(name: string, onError?: (message: string) => void) {
  try {
    const { publicKey } = await options('/settings/passkeys/options')
    const credential = await navigator.credentials.create({
      publicKey: PublicKeyCredential.parseCreationOptionsFromJSON(publicKey),
    })
    if (credential) router.post('/settings/passkeys', { name, credential: (credential as PublicKeyCredential).toJSON() }, { preserveScroll: true })
  } catch (e) {
    if (!cancelled(e)) onError?.('This browser could not create a passkey.')
  }
}

/** Passwordless sign-in, from the login page. */
export async function signIn() {
  try {
    const credential = await get('/passkeys/login/options')
    if (credential) router.post('/passkeys/login', { credential: credential.toJSON() })
  } catch (e) {
    if (!cancelled(e)) throw e
  }
}

/** The second factor, from the two-factor challenge page. `trust`: skip the challenge in this browser for 30 days. */
export async function secondFactor(trust = false) {
  try {
    const credential = await get('/two-factor-challenge/passkey/options')
    if (credential) router.post('/two-factor-challenge/passkey', { credential: credential.toJSON(), trust })
  } catch (e) {
    if (!cancelled(e)) throw e
  }
}
