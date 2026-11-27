<script setup lang="ts">
import { useForm } from '@inertiajs/vue3'

const props = defineProps<{ token: string; email: string }>()

const form = useForm({ token: props.token, email: props.email, password: '', password_confirmation: '' })
</script>

<template>
  <main>
    <h1>Choose a new password</h1>
    <form @submit.prevent="form.post('/reset-password', { onFinish: () => form.reset('password', 'password_confirmation') })">
      <label>
        Email
        <input v-model="form.email" type="email" autocomplete="username" required />
      </label>
      <p v-if="form.errors.email">{{ form.errors.email }}</p>

      <label>
        New password
        <input v-model="form.password" type="password" autocomplete="new-password" required />
      </label>
      <p v-if="form.errors.password">{{ form.errors.password }}</p>

      <label>
        Confirm password
        <input v-model="form.password_confirmation" type="password" autocomplete="new-password" required />
      </label>
      <p v-if="form.errors.password_confirmation">{{ form.errors.password_confirmation }}</p>

      <button type="submit" :disabled="form.processing">Reset password</button>
    </form>
  </main>
</template>
