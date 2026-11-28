<script setup lang="ts">
import { ref } from 'vue'
import { useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AuthLayout from '../../layouts/AuthLayout.vue'

const backup = ref(false)
const form = useForm({ code: '', backup_code: '', trust: false })
const submit = () =>
  form
    .transform((d) => (backup.value ? { backup_code: d.backup_code, trust: d.trust } : { code: d.code, trust: d.trust }))
    .post('/two-factor-challenge', { onFinish: () => form.reset('code', 'backup_code') })
const toggle = () => {
  backup.value = !backup.value
  form.clearErrors()
}
</script>

<template>
  <AuthLayout
    title="Two-factor authentication"
    :subtitle="backup ? 'Enter one of your backup codes.' : 'Enter the 6-digit code from your authenticator app.'"
  >
    <form @submit.prevent="submit">
      <Field v-if="!backup" v-model="form.code" label="Code" autocomplete="one-time-code" :error="form.errors.code" autofocus />
      <Field v-else v-model="form.backup_code" label="Backup code" autocomplete="off" :error="form.errors.backup_code ?? form.errors.code" autofocus />
      <label class="check"><input v-model="form.trust" type="checkbox" /> Trust this browser for 30 days</label>
      <button class="btn btn-ink btn-lg btn-block" type="submit" :disabled="form.processing">Log in</button>
    </form>
    <template #foot>
      <button class="link" type="button" @click="toggle">{{ backup ? 'Use your authenticator app' : 'Use a backup code' }}</button>
    </template>
  </AuthLayout>
</template>
