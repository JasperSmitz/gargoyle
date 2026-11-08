// Compile-time helpers that check a JSON import against a contract type.
//
// TypeScript infers JSON string values as `string` (never as literals) and gives no way to import
// JSON `as const`, so a plain `json satisfies Overview` cannot work. Instead, `Conforms<J, T>`
// checks the JSON in two directions:
//   - `Widen<T>`: every contract field is present with the right primitive type and nullability
//     (literal unions widened to `string`; their values are checked at runtime in the tests).
//   - `Exact<J, T>`: the JSON has no keys the contract lacks, at any depth.

/** `T` with string-literal unions widened to `string` and tuples to arrays. */
export type Widen<T> = T extends string
  ? string
  : T extends number
    ? number
    : T extends boolean
      ? boolean
      : T extends null
        ? null
        : T extends readonly (infer E)[]
          ? Widen<E>[]
          : { [K in keyof T]: Widen<T[K]> };

/** `J` with every key that `T` does not declare mapped to `never`, recursively. */
export type Exact<J, T> = J extends readonly (infer JE)[]
  ? T extends readonly (infer TE)[]
    ? readonly Exact<JE, TE>[]
    : never
  : J extends object
    ? { [K in keyof J]: K extends keyof T ? Exact<J[K], NonNullable<T[K]>> : never }
    : J;

/** Use as `json satisfies Conforms<typeof json, T>`. */
export type Conforms<J, T> = Widen<T> & Exact<J, T>;
