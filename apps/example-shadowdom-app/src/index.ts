// Define all the island web components here.

import './app-shell.js'
import './app-item.js'
import './app-header.js'
import './button.js'

new PerformanceObserver((list) => {
  list.getEntries().forEach((entry) => {
    console.log(entry.name, entry.duration, 'ms')
  })
}).observe({ type: 'measure', buffered: true })
new PerformanceObserver((list) => {
  list.getEntries().forEach((entry) => {
    console.log(entry.name, entry.startTime, 'ms')
  })
}).observe({ type: 'paint', buffered: true })
