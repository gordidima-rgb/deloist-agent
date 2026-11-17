<script>
  // Только для SettingsForm.test.js: карточка с одной строкой поля.
  import SettingsCard from './SettingsCard.svelte';
  import FieldRow from './FieldRow.svelte';
  import TextInput from './TextInput.svelte';
</script>

<div class="k-sform">
  <SettingsCard title="Section" description="Hint line" icon="bot">
    <FieldRow block label="Field" icon="target" tip="Tip">
      {#snippet children(f)}<TextInput id={f.id} value="x" />{/snippet}
    </FieldRow>
  </SettingsCard>
</div>
