<script setup lang="ts">
import { router, useForm } from '@inertiajs/vue3'
import { ref } from 'vue'

const props = defineProps<{
  base: string
  kind: 'team' | 'organization'
  name: string
  canRename: boolean
  canLeave: boolean
  canDelete: boolean
  leaveNote: string
  deleteNote: string
}>()

const opts = { preserveScroll: true }
const rename = useForm({ name: props.name })
const typed = ref('')
const leave = () => confirm(`Leave ${props.name}?`) && router.delete(`${props.base}/membership`)
const destroy = () => typed.value === props.name && router.delete(props.base)
</script>

<template>
  <section v-if="canRename" aria-labelledby="settings-h">
    <div class="strip"><h2 id="settings-h">Settings</h2></div>
    <div class="setting">
      <div>
        <h3>Name</h3>
        <p>Shown to members and in invitation emails.</p>
      </div>
      <div>
        <form @submit.prevent="rename.put(base, opts)">
          <input v-model="rename.name" class="input" :aria-label="`${kind} name`" required maxlength="80" :aria-invalid="rename.errors.name ? 'true' : undefined" />
          <p v-if="rename.errors.name" class="error">{{ rename.errors.name }}</p>
          <button class="btn btn-line" type="submit" :disabled="rename.processing || !rename.isDirty">{{ rename.processing ? 'Saving…' : 'Save' }}</button>
        </form>
      </div>
    </div>
  </section>

  <section v-if="canLeave || canDelete" aria-labelledby="danger-h">
    <div class="strip strip-danger"><h2 id="danger-h">Danger zone</h2></div>
    <div v-if="canLeave" class="setting">
      <div>
        <h3>Leave {{ kind }}</h3>
        <p>{{ leaveNote }}</p>
      </div>
      <div><button class="btn btn-danger-line" type="button" @click="leave">Leave {{ kind }}</button></div>
    </div>
    <div v-if="canDelete" class="setting">
      <div>
        <h3>Delete {{ kind }}</h3>
        <p>{{ deleteNote }}</p>
      </div>
      <div>
        <form @submit.prevent="destroy">
          <label class="hint" for="confirm-name">Type <code>{{ name }}</code> to confirm.</label>
          <input id="confirm-name" v-model="typed" class="input" autocomplete="off" spellcheck="false" />
          <button class="btn btn-danger" type="submit" :disabled="typed !== name">Delete {{ kind }}</button>
        </form>
      </div>
    </div>
  </section>
  <div class="tail" />
</template>
