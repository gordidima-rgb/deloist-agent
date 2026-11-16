const types = {
  image: 'jpg jpeg png gif webp svg bmp ico avif tiff tif heic',
  audio: 'wav mp3 m4a aac flac ogg opus wma',
  video: 'mp4 mov webm avi mkv m4v mpeg mpg',
  document: 'doc docx odt rtf pages',
  pdf: 'pdf',
  spreadsheet: 'xls xlsx ods csv tsv numbers',
  presentation: 'ppt pptx odp key',
  archive: 'zip rar 7z tar gz bz2 xz tgz zst',
  code: 'js jsx ts tsx py rs go java c h cpp hpp cs css scss html htm json yaml yml toml xml sh ps1 sql svelte vue php rb swift kt',
  text: 'txt md markdown log rst tex',
};

const byExtension = Object.fromEntries(Object.entries(types).flatMap(([type, extensions]) =>
  extensions.split(' ').map((extension) => [extension, type])));

export function fileType(name) {
  const base = String(name ?? '').split(/[\\/]/).pop().toLowerCase();
  const dot = base.lastIndexOf('.');
  if (dot <= 0 || dot === base.length - 1) return 'unknown';
  return byExtension[base.slice(dot + 1)] || 'unknown';
}
