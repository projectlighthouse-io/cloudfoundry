<script setup lang="ts">
import { usePage } from '@inertiajs/vue3'
import { computed } from 'vue'

// `social` is a shared prop: the providers registered with socialite::routes.
const labels: Record<string, string> = {
  github: 'GitHub', gitlab: 'GitLab', google: 'Google', reddit: 'Reddit', twitter: 'X', auth0: 'Auth0',
}
const providers = computed(() => ((usePage().props.social as string[] | undefined) ?? []))
</script>

<template>
  <div v-if="providers.length">
    <!-- Plain links: the provider redirect is a full page visit, not an Inertia one. -->
    <a v-for="p in providers" :key="p" :href="`/auth/${p}`">Continue with {{ labels[p] ?? p }}</a>
  </div>
</template>
