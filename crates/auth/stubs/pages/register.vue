<script setup lang="ts">
import { Link, useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AuthLayout from '../../layouts/AuthLayout.vue'

const form = useForm({ name: '', email: '', password: '', password_confirmation: '' })
const submit = () => form.post('/register', { onFinish: () => form.reset('password', 'password_confirmation') })
</script>

<template>
  <AuthLayout title="Create an account" subtitle="Use at least 8 characters for your password.">
    <form @submit.prevent="submit">
      <Field v-model="form.name" label="Name" autocomplete="name" :error="form.errors.name" autofocus />
      <Field v-model="form.email" label="Email" type="email" autocomplete="username" :error="form.errors.email" />
      <Field v-model="form.password" label="Password" type="password" autocomplete="new-password" :error="form.errors.password" />
      <Field v-model="form.password_confirmation" label="Confirm password" type="password" autocomplete="new-password" :error="form.errors.password_confirmation" />
      <button class="btn btn-ink btn-lg btn-block" type="submit" :disabled="form.processing">Create account</button>
    </form>
    <template #foot>Already registered? <Link href="/login">Log in</Link></template>
  </AuthLayout>
</template>
