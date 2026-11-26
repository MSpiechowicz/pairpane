<script lang="ts">
  import type { QrImage } from '$lib/ipc/bindings/QrImage';

  let { qr }: { qr: QrImage } = $props();
</script>

<!-- The path holds digits and M, h, v, z only (the backend makes it); it is drawn black on a white square with a quiet zone of 4 modules. -->
<svg class="qr" viewBox="-4 -4 {qr.size + 8} {qr.size + 8}" role="img" aria-label="QR code of this device's address" shape-rendering="crispEdges">
  <rect x="-4" y="-4" width={qr.size + 8} height={qr.size + 8} fill="#fff" />
  <path d={qr.path} fill="#000" />
</svg>

<style>
  .qr {
    width: 160px;
    height: 160px;
    flex: none;
    border-radius: 6px;
  }
</style>
