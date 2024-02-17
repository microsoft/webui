export const html = (strings: readonly string[] | ArrayLike<string>, ...values: unknown[]) =>
  String.raw({ raw: strings }, ...values)
