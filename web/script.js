const canvas = document.querySelector("#canvas");
const context = canvas.getContext("2d");

const data = context.createImageData(canvas.width, canvas.height);
for (let i = 0; i < data.data.length; i += 4) {
  data.data[i] = 0;
  data.data[i + 1] = 0;
  data.data[i + 2] = 0;
  data.data[i + 3] = 255;
}
context.putImageData(data, 0, 0);