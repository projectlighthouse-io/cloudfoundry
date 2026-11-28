<script setup lang="ts">
import type { EmailPage } from '../../types/EmailPage'
import { router, useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AppLayout from '../../layouts/AppLayout.vue'

defineProps<EmailPage>()

const form = useForm({ email: '' })
const submit = () => form.post('/settings/email', { preserveScroll: true, onSuccess: () => form.reset() })
const resend = () => router.post('/settings/email/verification-notification', {}, { preserveScroll: true })
</script>

<template>
  <AppLayout title="Settings" :user="email">
    <header class="phead">
      <div class="phead-id">
        <div>
          <h1>Settings</h1>
          <p class="sub">Signed in as {{ email }}.</p>
        </div>
      </div>
    </header>

    <section aria-labelledby="email-h">
      <div class="strip">
        <h2 id="email-h">Email address</h2>
        <span class="meta">{{ verified ? 'Verified' : 'Not verified' }}</span>
      </div>
      <p v-if="status" class="flash flash-ok" role="status">{{ status }}</p>

      <div v-if="!verified" class="setting">
        <div>
          <h3>Verify {{ email }}</h3>
          <p>Open the link we sent you. It works for an hour.</p>
        </div>
        <div><button class="btn btn-line" type="button" @click="resend">Send a new link</button></div>
      </div>

      <div class="setting">
        <div>
          <h3>Change email address</h3>
          <p>We send a link to the new address; it replaces {{ email }} once you open it. You confirm it is you first.</p>
        </div>
        <form @submit.prevent="submit">
          <Field v-model="form.email" label="New email" type="email" autocomplete="email" :error="form.errors.email" />
          <button class="btn btn-ink" type="submit" :disabled="form.processing">Send link</button>
        </form>
      </div>
    </section>
    <div class="tail" />
  </AppLayout>
</template>
