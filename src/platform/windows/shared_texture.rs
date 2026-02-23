use std::sync::Arc;

use tracing::{debug, warn};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_QUERY_DESC, D3D11_QUERY_EVENT, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, ID3D11Device, ID3D11DeviceContext, ID3D11Query, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIKeyedMutex, IDXGIResource1,
};
use windows::core::Interface;

const POOL_SIZE: usize = 3;

/// A single shared texture slot in the pool.
pub struct SharedTextureSlot {
    texture: ID3D11Texture2D,
    keyed_mutex: IDXGIKeyedMutex,
    pub shared_handle: HANDLE,
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
}

impl Drop for SharedTextureSlot {
    fn drop(&mut self) {
        // NT handles from CreateSharedHandle must be explicitly closed.
        unsafe {
            let _ = CloseHandle(self.shared_handle);
        }
    }
}

impl SharedTextureSlot {
    /// Read the shared texture's pixel data back to CPU as tightly-packed RGBA bytes.
    ///
    /// Creates a staging texture, copies the GPU data into it, maps it.
    ///
    /// Thread-safety: the `Arc::strong_count` check in
    /// `SharedTexturePool::acquire_and_copy` prevents the capture handler from
    /// reusing a slot while the UI holds a reference, so no concurrent writer
    /// can exist when this method is called.
    pub fn read_pixels(&self) -> Result<Vec<u8>, SharedTextureError> {
        let device = unsafe { self.texture.GetDevice()? };
        let context = unsafe { device.GetImmediateContext()? };

        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: self.width,
            Height: self.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };

        let mut staging: Option<ID3D11Texture2D> = None;
        unsafe {
            device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
        }
        let staging = staging.ok_or_else(|| {
            SharedTextureError::Windows(windows::core::Error::from_hresult(windows::core::HRESULT(
                -1,
            )))
        })?;

        unsafe {
            self.keyed_mutex.AcquireSync(0, u32::MAX)?;
        }

        // GPU-side copy on the same device
        unsafe {
            context.CopyResource(&staging, &self.texture);
        }

        unsafe {
            self.keyed_mutex.ReleaseSync(0)?;
        }

        // Map with D3D11_MAP_READ + flags=0 blocks until the GPU copy completes.
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }

        // Copy mapped data into an owned Vec before unmapping.
        let row_pitch = mapped.RowPitch as usize;
        let row_bytes = (self.width as usize) * 4;
        let mut rgba = vec![0u8; row_bytes * self.height as usize];

        for y in 0..self.height as usize {
            let src = unsafe {
                std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * row_pitch),
                    row_bytes,
                )
            };
            rgba[y * row_bytes..(y + 1) * row_bytes].copy_from_slice(src);
        }

        unsafe {
            context.Unmap(&staging, 0);
        }

        Ok(rgba)
    }
}

// The ID3D11Texture2D and IDXGIKeyedMutex COM references use atomic
// AddRef/Release.  HANDLE is a pointer-sized integer.
unsafe impl Send for SharedTextureSlot {}
unsafe impl Sync for SharedTextureSlot {}

#[derive(Debug)]
pub enum SharedTextureError {
    Windows(windows::core::Error),
    #[allow(dead_code)]
    NoAvailableSlot,
}

impl std::fmt::Display for SharedTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Windows(e) => write!(f, "Windows error: {e}"),
            Self::NoAvailableSlot => write!(f, "No available shared texture slot"),
        }
    }
}

impl std::error::Error for SharedTextureError {}

impl From<windows::core::Error> for SharedTextureError {
    fn from(e: windows::core::Error) -> Self {
        Self::Windows(e)
    }
}

/// Pool of shared D3D11 textures for zero-copy frame delivery.
///
/// Manages a ring of [`POOL_SIZE`] textures, each created with
/// `D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX`
/// so that an NT shared handle can be obtained and later opened by the DX12
/// render device via `ID3D12Device::OpenSharedHandle`.
pub struct SharedTexturePool {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    slots: Vec<Arc<SharedTextureSlot>>,
    current_width: u32,
    current_height: u32,
    event_query: ID3D11Query,
}

impl SharedTexturePool {
    /// Create a new pool.  Call once when capture starts.
    ///
    /// `source_texture` is used to obtain the D3D11 device and context.
    pub fn new(source_texture: &ID3D11Texture2D) -> Result<Self, SharedTextureError> {
        let device = unsafe { source_texture.GetDevice()? };
        let context = unsafe { device.GetImmediateContext()? };

        let query_desc = D3D11_QUERY_DESC {
            Query: D3D11_QUERY_EVENT,
            MiscFlags: 0,
        };
        let mut event_query: Option<ID3D11Query> = None;
        unsafe { device.CreateQuery(&query_desc, Some(&mut event_query as *mut _))? };
        let event_query = event_query.ok_or_else(|| {
            SharedTextureError::Windows(windows::core::Error::from_hresult(windows::core::HRESULT(
                -1,
            )))
        })?;

        Ok(Self {
            device,
            context,
            slots: Vec::new(),
            current_width: 0,
            current_height: 0,
            event_query,
        })
    }

    /// Copy the WGC frame texture into an available pool slot and return it.
    ///
    /// Returns `Ok(None)` if all slots are currently in use (backpressure).
    pub fn acquire_and_copy(
        &mut self,
        source_texture: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<Option<Arc<SharedTextureSlot>>, SharedTextureError> {
        // Recreate pool when resolution changes
        if width != self.current_width || height != self.current_height {
            self.recreate_pool(width, height)?;
        }

        // Find an available slot (only the pool holds a reference)
        let slot_idx = self.slots.iter().position(|s| Arc::strong_count(s) == 1);
        let Some(idx) = slot_idx else {
            return Ok(None);
        };

        // Acquire keyed mutex before writing to the shared texture.
        // The mutex starts released with key 0; since we always release with
        // key 0, subsequent acquires succeed immediately.
        unsafe {
            self.slots[idx].keyed_mutex.AcquireSync(0, u32::MAX)?;
        }

        // GPU-to-GPU copy on the same adapter
        unsafe {
            self.context
                .CopyResource(&self.slots[idx].texture, source_texture);
        }

        // Release keyed mutex — this provides a GPU memory barrier that
        // ensures the copy is visible when DX12 opens the shared handle.
        unsafe {
            self.slots[idx].keyed_mutex.ReleaseSync(0)?;
        }

        // Flush the command buffer and wait for GPU completion.
        unsafe {
            self.context.End(&self.event_query);
            self.context.Flush();
        }

        // Spin-wait for the GPU to finish the copy.
        // CopyResource on the same adapter typically completes in microseconds.
        let mut done = 0u32;
        loop {
            let hr = unsafe {
                self.context.GetData(
                    &self.event_query,
                    Some(std::ptr::addr_of_mut!(done).cast()),
                    4,
                    0,
                )
            };
            if done != 0 || hr.is_err() {
                break;
            }
            std::hint::spin_loop();
        }

        Ok(Some(Arc::clone(&self.slots[idx])))
    }

    fn recreate_pool(&mut self, width: u32, height: u32) -> Result<(), SharedTextureError> {
        debug!(width, height, "Recreating shared texture pool");
        self.slots.clear();

        for _ in 0..POOL_SIZE {
            let (texture, keyed_mutex, shared_handle) =
                create_shared_texture(&self.device, width, height)?;
            self.slots.push(Arc::new(SharedTextureSlot {
                texture,
                keyed_mutex,
                shared_handle,
                width,
                height,
            }));
        }

        self.current_width = width;
        self.current_height = height;
        Ok(())
    }
}

impl Drop for SharedTexturePool {
    fn drop(&mut self) {
        // Check for leaked slot references
        for (i, slot) in self.slots.iter().enumerate() {
            let count = Arc::strong_count(slot);
            if count > 1 {
                warn!(
                    slot = i,
                    refcount = count,
                    "Shared texture slot still in use during pool drop"
                );
            }
        }
    }
}

fn create_shared_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<(ID3D11Texture2D, IDXGIKeyedMutex, HANDLE), SharedTextureError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0)
            as u32,
    };

    let mut texture: Option<ID3D11Texture2D> = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture as *mut _))? };
    let texture = texture.ok_or_else(|| {
        SharedTextureError::Windows(windows::core::Error::from_hresult(windows::core::HRESULT(
            -1,
        )))
    })?;

    // Get the keyed mutex interface for synchronization.
    let keyed_mutex: IDXGIKeyedMutex = texture.cast()?;

    // Obtain an NT shared handle via IDXGIResource1::CreateSharedHandle.
    // Unlike legacy handles, NT handles must be explicitly closed via CloseHandle.
    let dxgi_resource1: IDXGIResource1 = texture.cast()?;
    let shared_handle = unsafe {
        dxgi_resource1.CreateSharedHandle(
            None,
            DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0,
            None,
        )?
    };

    debug!(
        width,
        height,
        ?shared_handle,
        "Created shared D3D11 texture with NT handle"
    );

    Ok((texture, keyed_mutex, shared_handle))
}
