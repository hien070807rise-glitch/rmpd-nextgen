use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub const RMPD_DSP_ABI_VERSION: u32 = 1;

/// C-ABI plugin function table matching `contracts/interfaces/dsp_plugin_abi.h`.
#[repr(C)]
pub struct RmpdDspPluginVTable {
    pub abi_version: u32,
    pub id: *const c_char,
    pub name: *const c_char,
    pub create: unsafe extern "C" fn(sample_rate: u32, channels: u16) -> *mut c_void,
    pub destroy: unsafe extern "C" fn(instance: *mut c_void),
    pub process_f32: unsafe extern "C" fn(
        instance: *mut c_void,
        in_interleaved: *const f32,
        out_interleaved: *mut f32,
        frames: usize,
    ),
    pub reset: unsafe extern "C" fn(instance: *mut c_void),
}

// Function pointers and static C string identifiers in the ABI vtable are immutable across threads.
unsafe impl Sync for RmpdDspPluginVTable {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    AbiVersionMismatch { expected: u32, found: u32 },
    NullInstance,
    MismatchedBufferLength,
    PluginPanicked,
}

pub struct HostedDspPlugin {
    vtable: &'static RmpdDspPluginVTable,
    instance: *mut c_void,
    channels: u16,
}

unsafe impl Send for HostedDspPlugin {}

impl HostedDspPlugin {
    pub fn load(
        vtable: &'static RmpdDspPluginVTable,
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self, PluginError> {
        if vtable.abi_version != RMPD_DSP_ABI_VERSION {
            return Err(PluginError::AbiVersionMismatch {
                expected: RMPD_DSP_ABI_VERSION,
                found: vtable.abi_version,
            });
        }
        let instance = unsafe { (vtable.create)(sample_rate, channels) };
        if instance.is_null() {
            return Err(PluginError::NullInstance);
        }
        Ok(Self {
            vtable,
            instance,
            channels,
        })
    }

    pub fn id(&self) -> &str {
        if self.vtable.id.is_null() {
            return "unknown";
        }
        unsafe { CStr::from_ptr(self.vtable.id) }
            .to_str()
            .unwrap_or("invalid-utf8")
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), PluginError> {
        if input.len() != output.len() || input.len() % (self.channels.max(1) as usize) != 0 {
            return Err(PluginError::MismatchedBufferLength);
        }
        let frames = input.len() / (self.channels.max(1) as usize);
        let inst = self.instance;
        let vt = self.vtable;
        let res = catch_unwind(AssertUnwindSafe(|| unsafe {
            (vt.process_f32)(inst, input.as_ptr(), output.as_mut_ptr(), frames);
        }));
        if res.is_err() {
            return Err(PluginError::PluginPanicked);
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        unsafe { (self.vtable.reset)(self.instance) };
    }
}

impl Drop for HostedDspPlugin {
    fn drop(&mut self) {
        if !self.instance.is_null() {
            unsafe { (self.vtable.destroy)(self.instance) };
            self.instance = std::ptr::null_mut();
        }
    }
}

// Built-in reference C-ABI soft-limiter / gain plugin
struct SoftLimiterState {
    channels: u16,
}

unsafe extern "C" fn ref_create(_sample_rate: u32, channels: u16) -> *mut c_void {
    Box::into_raw(Box::new(SoftLimiterState { channels })) as *mut c_void
}

unsafe extern "C" fn ref_destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(unsafe { Box::from_raw(instance as *mut SoftLimiterState) });
    }
}

unsafe extern "C" fn ref_process_f32(
    instance: *mut c_void,
    in_interleaved: *const f32,
    out_interleaved: *mut f32,
    frames: usize,
) {
    if instance.is_null() || in_interleaved.is_null() || out_interleaved.is_null() {
        return;
    }
    let st = unsafe { &*(instance as *const SoftLimiterState) };
    let total = frames * (st.channels as usize);
    let inp = unsafe { std::slice::from_raw_parts(in_interleaved, total) };
    let out = unsafe { std::slice::from_raw_parts_mut(out_interleaved, total) };
    for (o, &i) in out.iter_mut().zip(inp.iter()) {
        *o = i.clamp(-1.0, 1.0);
    }
}

unsafe extern "C" fn ref_reset(_instance: *mut c_void) {}

static REF_ID: &[u8] = b"rmpd.soft_limiter\0";
static REF_NAME: &[u8] = b"RMPD Reference Soft Limiter\0";

static REF_VTABLE: RmpdDspPluginVTable = RmpdDspPluginVTable {
    abi_version: RMPD_DSP_ABI_VERSION,
    id: REF_ID.as_ptr() as *const c_char,
    name: REF_NAME.as_ptr() as *const c_char,
    create: ref_create,
    destroy: ref_destroy,
    process_f32: ref_process_f32,
    reset: ref_reset,
};

pub fn reference_soft_limiter_vtable() -> &'static RmpdDspPluginVTable {
    &REF_VTABLE
}

#[cfg(test)]
mod tests {
    use super::*;

    static BAD_ABI_VTABLE: RmpdDspPluginVTable = RmpdDspPluginVTable {
        abi_version: 999,
        id: REF_ID.as_ptr() as *const c_char,
        name: REF_NAME.as_ptr() as *const c_char,
        create: ref_create,
        destroy: ref_destroy,
        process_f32: ref_process_f32,
        reset: ref_reset,
    };

    #[test]
    fn test_abi_mismatch_rejected() {
        let err = HostedDspPlugin::load(&BAD_ABI_VTABLE, 48_000, 2).err().unwrap();
        assert_eq!(
            err,
            PluginError::AbiVersionMismatch {
                expected: 1,
                found: 999
            }
        );
    }

    #[test]
    fn test_reference_c_abi_plugin_process() {
        let mut host = HostedDspPlugin::load(reference_soft_limiter_vtable(), 48_000, 2).unwrap();
        assert_eq!(host.id(), "rmpd.soft_limiter");
        let inp = [-1.5f32, 0.25, 0.5, 2.0];
        let mut out = [0.0f32; 4];
        host.process(&inp, &mut out).unwrap();
        assert_eq!(out, [-1.0, 0.25, 0.5, 1.0]);
    }
}
