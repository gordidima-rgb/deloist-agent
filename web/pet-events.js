// Один поток событий на всех питомцев. У каждого своя страница, но профиль общий, а браузер держит к одному
// серверу не больше шести соединений: с отдельным потоком на страницу седьмой питомец не дождался бы ответа вообще.
const ports = new Set();

function broadcast(message) {
  for (const port of ports) port.postMessage(message);
}

const stream = new EventSource('/api/events');
stream.addEventListener('message', (event) => broadcast(event.data));
stream.addEventListener('error', () => broadcast(null));

self.addEventListener('connect', (event) => {
  const port = event.ports[0];
  ports.add(port);
  port.start();
});
