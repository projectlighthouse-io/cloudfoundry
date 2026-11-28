<script setup lang="ts">
import { Link } from '@inertiajs/vue3'
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'

const props = defineProps<{ email: string }>()

// The account menu. Add a link per page you publish, e.g. { href: '/teams', label: 'Teams' }.
const links = [
  { href: '/', label: 'Home' },
  { href: '/settings/password', label: 'Settings' },
]

const open = ref(false)
const root = ref<HTMLElement | null>(null)
const trigger = ref<HTMLButtonElement | null>(null)
const initial = props.email.charAt(0).toUpperCase()

const close = (focus = false) => {
  open.value = false
  if (focus) trigger.value?.focus()
}
const toggle = async () => {
  open.value = !open.value
  if (open.value) {
    await nextTick()
    root.value?.querySelector<HTMLElement>('[role=menuitem]')?.focus()
  }
}
const onDoc = (e: MouseEvent) => {
  if (open.value && root.value && !root.value.contains(e.target as Node)) close()
}
const onKey = (e: KeyboardEvent) => {
  if (!open.value) return
  if (e.key === 'Escape') return close(true)
  if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
  e.preventDefault()
  const items = [...(root.value?.querySelectorAll<HTMLElement>('[role=menuitem]') ?? [])]
  const i = items.indexOf(document.activeElement as HTMLElement)
  items[(i + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]?.focus()
}
onMounted(() => document.addEventListener('click', onDoc))
onBeforeUnmount(() => document.removeEventListener('click', onDoc))
</script>

<template>
  <div ref="root" class="umenu" @keydown="onKey">
    <button ref="trigger" type="button" class="utrigger" aria-haspopup="menu" :aria-expanded="open" @click="toggle">
      <span class="avatar" aria-hidden="true">{{ initial }}</span>
      <span class="uemail">{{ email }}</span>
      <svg class="ic chev" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
    </button>
    <div v-if="open" class="upanel" role="menu" aria-label="Account">
      <div class="uhead">
        <p class="uhead-label">Signed in as</p>
        <p class="uhead-email">{{ email }}</p>
      </div>
      <Link v-for="l in links" :key="l.href" class="uitem" role="menuitem" :href="l.href" @click="close()">{{ l.label }}</Link>
      <div class="usep" role="separator" />
      <Link class="uitem" role="menuitem" href="/logout" method="post" as="button" @click="close()">Log out</Link>
    </div>
  </div>
</template>
