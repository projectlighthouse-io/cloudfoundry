<script setup lang="ts">
import type { ConfirmPasswordPage } from '../../types/ConfirmPasswordPage'
import { useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AuthLayout from '../../layouts/AuthLayout.vue'

defineProps<ConfirmPasswordPage>()

const form = useForm({ password: '' })
const submit = () => form.post('/settings/confirm-password', { onFinish: () => form.reset() })
</script>

<template>
  <AuthLayout title="Confirm your password" subtitle="This is a secure area. Confirm your password to continue.">
    <form @submit.prevent="submit">
      <p class="sub">Signed in as {{ email }}.</p>
      <Field v-model="form.password" label="Password" type="password" autocomplete="current-password" :error="form.errors.password" autofocus />
      <button class="btn btn-ink btn-lg btn-block" type="submit" :disabled="form.processing">Confirm</button>
    </form>
  </AuthLayout>
</template>
