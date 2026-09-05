# Butaca

Cliente nativo de [Jellyfin](https://jellyfin.org/) para televisores LG webOS. Nada de página
web: la interfaz se dibuja directamente en la GPU a 60 fps y el vídeo lo decodifica el propio
hardware de la televisión.

Un sillón en el que sentarse a ver lo que hay en tu servidor.

## Objetivo

Sacar el máximo rendimiento posible a televisores LG **"no tan modernos"**. Los modelos con
webOS 5.x y anteriores arrastran un Chromium viejo y lento: las apps web oficiales se quedan
cortas justo donde no deberían, y no hay forma de que el navegador llegue a los 60 fps estables
de una interfaz nativa.

Butaca ataca ese problema de raíz: tira el navegador, dibuja sobre la GPU y entrega el vídeo al
mismo silicio que usan las apps integradas. Sin Chromium, sin JavaScript, sin web view.

## Qué hay hecho

Portado sobre [plx-native](https://github.com/GLinnik21/plx-native), sustituyendo el backend
Plex por el de Jellyfin:

- Interfaz nativa (Rust) a 60 fps sobre la TV, con escenas de regresión que lo miden en el
  propio televisor en vez de confiar en ello.
- Login de Jellyfin en pantalla: URL, usuario y contraseña, sin configuración previa.
- Navegación de librerías, detalle, perfiles y búsqueda cableadas a la API de Jellyfin.
- Playback directo (H.264 / HEVC) por la tubería de vídeo nativa de la TV.
- Adaptación de la identidad propia: nombre "Butaca", iconos y texto de pantalla.

*Trabajo en curso: se sigue iterando; cada versión se acumula en el árbol de trabajo.*

## Agradecimiento

Este proyecto es un fork de [plx-native](https://github.com/GLinnik21/plx-native), el cliente
nativo Plex para webOS de [Gleb Linnik](https://github.com/GLinnik21). El motor de renderizado,
la capa de host y el pipeline de vídeo sobre los que se apoya Butaca son suyos, y la cantidad de
horas que le habrá costado llegar hasta ahí es evidente. Le estamos profundamente agradecidos, y
esto no existiría sin su trabajo.

## Licencia

[MIT](LICENSE). El copyright original pertenece a Gleb Linnik (ver LICENSE); los cambios y
adaptaciones de Butaca se publican bajo los mismos términos. La marca "Butaca" y el fork en sí no
están afiliados, respaldados ni patrocinados por Jellyfin, LG ni Gleb Linnik.

**Unofficial client.** "Jellyfin", "LG" y "webOS" son marcas de sus respectivos propietarios;
donde aparecen, identifican el servicio o la plataforma con la que trabaja la aplicación.
