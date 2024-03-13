export function attr(target: HTMLElement, propertyKey: string) {
  Object.defineProperty(target, propertyKey, {
    get: function() {
      return this.getAttribute(propertyKey)
    },
    set: function(newValue: string) {
      const oldValue = this.getAttribute(propertyKey)
      this.setAttribute(propertyKey, newValue)
      if (this.hydrated && this[`${propertyKey}Changed`]) {
        this[`${propertyKey}Changed`](oldValue, newValue)
      }
    },
  })
}
