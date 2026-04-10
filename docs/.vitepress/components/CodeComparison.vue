// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

<template>
  <div class="comparison-wrapper">
    <div class="comparison-col">
      <div class="comparison-label comparison-label--left">{{ leftLabel }}</div>
      <slot name="left" />
    </div>
    <div class="comparison-col">
      <div class="comparison-label comparison-label--right">{{ rightLabel }}</div>
      <slot name="right" />
    </div>
  </div>
</template>

<script setup>
defineProps({
  leftLabel: { type: String, default: 'React' },
  rightLabel: { type: String, default: 'WebUI' },
})
</script>
