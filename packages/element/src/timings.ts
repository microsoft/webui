import { hydrationTracker } from './custom-element.js'

const po = new PerformanceObserver((entryList) => {
  const navigationEntries = entryList.getEntriesByType('navigation')
  if (navigationEntries.length > 0) {
    console.log(`TTFB: ${(navigationEntries[0] as PerformanceResourceTiming).responseStart.toFixed(2)}ms`)
  }

  for (const entry of entryList.getEntriesByName('first-paint')) {
    console.log(`FP: ${entry.startTime.toFixed(2)}ms`)
  }

  for (const entry of entryList.getEntriesByName('first-contentful-paint')) {
    console.log(`FCP: ${entry.startTime.toFixed(2)}ms`)
    const fcpWidget = document.createElement('div')
    fcpWidget.style.position = 'fixed'
    fcpWidget.style.bottom = '0'
    fcpWidget.style.right = '0'
    fcpWidget.style.backgroundColor = 'white'
    fcpWidget.style.padding = '10px'
    fcpWidget.style.border = '1px solid black'
    fcpWidget.style.zIndex = '999'
    fcpWidget.textContent = `FCP: ${entry.startTime.toFixed(2)}ms`
    document.body.appendChild(fcpWidget)
  }

  for (const entry of entryList.getEntriesByType('largest-contentful-paint')) {
    console.log(`LCP: ${entry.startTime.toFixed(2)}ms`)

    const duration = hydrationTracker.reduce((acc, current) => {
      if (current.endTime === 0) {
        return acc
      }
      return acc + (current.endTime - current.startTime)
    }, 0)
    console.log(`Hydrated: ${duration.toFixed(2)}ms`)
  }
})

export function LogPerformanceTimings() {
  po.observe({ type: 'navigation', buffered: true })
  po.observe({ type: 'paint', buffered: true })
  po.observe({ type: 'largest-contentful-paint', buffered: true })
}
