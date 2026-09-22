/// <reference types="vite/client" />

/**
 * The three build-time prefill variables, declared rather than read off
 * `ImportMetaEnv`'s index signature.
 *
 * `vite/client` types that index signature as `any`, so
 * `import.meta.env.VITE_VPAY_BASE_URL` would be an `any` — which
 * `@typescript-eslint/no-unsafe-assignment` fails the lint on, and rightly:
 * an `any` flowing into `new VpayCheckout({...})` would defeat every check
 * that constructor makes. A declared member wins over the index signature,
 * so naming the three here is what makes `src/main.ts` type-safe.
 *
 * `string | undefined` and not `string`: a `VITE_` variable that was not set
 * is simply absent from the compiled bundle, and the screen has to cope with
 * all three being empty — which is the ordinary case.
 */
interface ImportMetaEnv {
  readonly VITE_VPAY_BASE_URL?: string;
  readonly VITE_VPAY_PUBLISHABLE_KEY?: string;
  readonly VITE_VPAY_SESSION_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
