<script setup lang="ts">
import { useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AuthLayout from '../../layouts/AuthLayout.vue'

const props = defineProps<{ token: string; email: string }>()

const form = useForm({ token: props.token, email: props.email, password: '', password_confirmation: '' })
const submit = () => form.post('/reset-password', { onFinish: () => form.reset('password', 'password_confirmation') })
</script>

<template>
  <AuthLayout title="Choose a new password" subtitle="Use at least 8 characters.">
    <form @submit.prevent="submit">
      <Field v-model="form.email" label="Email" type="email" autocomplete="username" :error="form.errors.email" />
      <Field v-model="form.password" label="New password" type="password" autocomplete="new-password" :error="form.errors.password" autofocus />
      <Field v-model="form.password_confirmation" label="Confirm password" type="password" autocomplete="new-password" :error="form.errors.password_confirmation" />
      <button class="btn btn-ink btn-lg btn-block" type="submit" :disabled="form.processing">Reset password</button>
    </form>
  </AuthLayout>
</template>
