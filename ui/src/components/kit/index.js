// Все компоненты кита «Угли». Экраны импортируют отсюда: import { Button, Select } from '../kit' (путь до папки kit).

// buttons
export { default as Button } from './buttons/Button.svelte';
export { default as IconButton } from './buttons/IconButton.svelte';

// inputs
export { default as TextInput } from './inputs/TextInput.svelte';
export { default as TextArea } from './inputs/TextArea.svelte';
export { default as Field } from './inputs/Field.svelte';
export { default as FieldRow } from './inputs/FieldRow.svelte';
export { default as Select } from './inputs/Select.svelte';
export { default as Slider } from './inputs/Slider.svelte';

// choice
export { default as Checkbox } from './choice/Checkbox.svelte';
export { default as RadioGroup } from './choice/RadioGroup.svelte';
export { default as Switch } from './choice/Switch.svelte';
export { default as Tabs } from './choice/Tabs.svelte';

// status
export { default as Badge } from './status/Badge.svelte';
export { default as Chip } from './status/Chip.svelte';
export { default as Spinner } from './status/Spinner.svelte';
export { default as Skeleton } from './status/Skeleton.svelte';
export { default as EmptyState } from './status/EmptyState.svelte';
export { default as LimitRing } from './status/LimitRing.svelte';

// overlay
export { default as Modal } from './overlay/Modal.svelte';
export { default as Toast } from './overlay/Toast.svelte';
export { default as ToastHost } from './overlay/ToastHost.svelte';
export { default as Tooltip } from './overlay/Tooltip.svelte';
export { default as InfoTip } from './overlay/InfoTip.svelte';
export { default as Menu } from './overlay/Menu.svelte';

// layout
export { default as Section } from './layout/Section.svelte';
export { default as Row } from './layout/Row.svelte';
export { default as SettingsCard } from './layout/SettingsCard.svelte';
export { default as SplitView } from './layout/SplitView.svelte';
export { default as Toolbar } from './layout/Toolbar.svelte';
export { default as ListItem } from './layout/ListItem.svelte';
export { default as ListRow } from './layout/ListRow.svelte';

// media
export { default as Avatar } from './media/Avatar.svelte';
export { default as Icon } from './media/Icon.svelte';

// Угли, этап B3: статичная метка, сетка-переключатель, состояние сохранения.
export { default as Tag } from './status/Tag.svelte';
export { default as SaveMark } from './status/SaveMark.svelte';
export { default as ChoiceGrid } from './choice/ChoiceGrid.svelte';
