<script setup lang="ts">
import { useForm } from '@inertiajs/vue3'

defineProps<{ status: string | null }>()

const form = useForm({ current_password: '', password: '', password_confirmation: '' })
const submit = () => form.post('/settings/password', { onFinish: () => form.reset() })
</script>

<template>
  <main>
    <h1>Change password</h1>
    <p v-if="status">{{ status }}</p>
    <form @submit.prevent="submit">
      <label>
        Current password
        <input v-model="form.current_password" type="password" autocomplete="current-password" required />
      </label>
      <p v-if="form.errors.current_password">{{ form.errors.current_password }}</p>

      <label>
        New password
        <input v-model="form.password" type="password" autocomplete="new-password" required />
      </label>
      <p v-if="form.errors.password">{{ form.errors.password }}</p>

      <label>
        Confirm new password
        <input v-model="form.password_confirmation" type="password" autocomplete="new-password" required />
      </label>

      <button type="submit" :disabled="form.processing">Change password</button>
    </form>
  </main>
</template>
