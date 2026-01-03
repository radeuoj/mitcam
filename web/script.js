const canvas = document.querySelector("#canvas");
canvas.width = 1280;
canvas.height = 720;
const context = canvas.getContext("2d");

const ws = new WebSocket(`ws://${window.location.host}/ws`);
clear();

ws.addEventListener("message", async (e) => {
  const imageData = new ImageData(new Uint8ClampedArray(await e.data.arrayBuffer()), 1280, 720);
  console.log(imageData.data);
  context.putImageData(imageData, 0, 0);
})

function clear() {
  const imageData = context.createImageData(canvas.width, canvas.height);
  for (let i = 0; i < imageData.data.length; i += 4) {
    imageData.data[i] = 0;
    imageData.data[i + 1] = 0;
    imageData.data[i + 2] = 0;
    imageData.data[i + 3] = 255;
  }
  context.putImageData(imageData, 0, 0);
}
