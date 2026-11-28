<script setup lang="ts">
defineProps<{ label: string; type?: string; error?: string; autocomplete?: string; autofocus?: boolean }>()
const model = defineModel<string>({ required: true })
const id = `f-${Math.random().toString(36).slice(2, 8)}`
</script>

<template>
  <div class="field">
    <div class="field-row">
      <label class="label" :for="id">{{ label }}</label>
      <slot name="aside" />
    </div>
    <input
      :id="id"
      v-model="model"
      class="input"
      :type="type ?? 'text'"
      :autocomplete="autocomplete"
      :autofocus="autofocus"
      :aria-invalid="error ? 'true' : undefined"
      :aria-describedby="error ? `${id}-err` : undefined"
      required
    />
    <p v-if="error" :id="`${id}-err`" class="error">{{ error }}</p>
  </div>
</template>
