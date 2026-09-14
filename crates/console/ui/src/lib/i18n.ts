/**
 * User-facing strings live here only (AGENTS.md: Rust sends codes, never display text).
 * Russian and English ship from day one (decision D17); the language follows the OS.
 */
type Args = (number | string)[]
type Entry = string | ((...args: Args) => string)

const en: Record<string, Entry> = {
  room: 'Classroom',
  loading: 'Loading…',
  noDevices: 'No student PCs yet',
  deviceCount: (total, live) => `${total} student PC${total === 1 ? '' : 's'} · ${live} live`,
  startWatching: 'Start watching',
  stopWatching: 'Stop watching',
  addPc: 'Add a PC',
  addFirst: 'Add a student PC first',
  dismiss: 'Dismiss',
  emptyTitle: 'No student PCs added yet',
  emptyBody: 'Add the first PC to see its screen here. You only need to do this once per computer.',
  thisConsole: 'This console:',
  capturing: 'Screens are being captured',
  notCapturing: 'Nothing is being captured',
  statusIdle: 'Not watching',
  statusConnecting: 'Connecting…',
  statusLive: 'Live',
  statusOffline: 'Offline',
  waitingFirst: 'Waiting for the first screen…',
  notWatching: 'Press “Start watching” to see this screen',
  pairTitle: 'Add a student PC',
  pairStep1: 'On the student PC, run this command:',
  pairStep2: 'Then read out this code when it asks:',
  pairWaiting: 'Waiting for the PC to connect…',
  pairDone: (id) => `PC ${id} added`,
  copy: 'Copy',
  copied: 'Copied',
  close: 'Close',
  cancel: 'Cancel',
  openFull: 'Open',
  screenOf: (id) => `Screen of PC ${id}`,
}

const ru: Record<string, Entry> = {
  room: 'Кабинет',
  loading: 'Загрузка…',
  noDevices: 'Компьютеров пока нет',
  deviceCount: (total, live) => `Компьютеров: ${total} · на связи: ${live}`,
  startWatching: 'Начать наблюдение',
  stopWatching: 'Остановить наблюдение',
  addPc: 'Добавить компьютер',
  addFirst: 'Сначала добавьте компьютер',
  dismiss: 'Закрыть',
  emptyTitle: 'Компьютеры ещё не добавлены',
  emptyBody: 'Добавьте первый компьютер, чтобы видеть его экран. Это нужно сделать один раз.',
  thisConsole: 'Этот компьютер учителя:',
  capturing: 'Экраны передаются',
  notCapturing: 'Экраны не передаются',
  statusIdle: 'Наблюдение выключено',
  statusConnecting: 'Подключение…',
  statusLive: 'На связи',
  statusOffline: 'Не в сети',
  waitingFirst: 'Ожидание первого кадра…',
  notWatching: 'Нажмите «Начать наблюдение», чтобы увидеть экран',
  pairTitle: 'Добавить компьютер ученика',
  pairStep1: 'На компьютере ученика выполните команду:',
  pairStep2: 'Затем продиктуйте этот код:',
  pairWaiting: 'Ожидание подключения компьютера…',
  pairDone: (id) => `Компьютер ${id} добавлен`,
  copy: 'Копировать',
  copied: 'Скопировано',
  close: 'Закрыть',
  cancel: 'Отмена',
  openFull: 'Открыть',
  screenOf: (id) => `Экран компьютера ${id}`,
}

const language = navigator.language.toLowerCase().startsWith('ru') ? ru : en

/** Looks up a string, calling it with `args` when it takes parameters. */
export function t(key: string, ...args: Args): string {
  const entry = language[key] ?? en[key]
  if (entry === undefined) return key
  return typeof entry === 'function' ? entry(...args) : entry
}
