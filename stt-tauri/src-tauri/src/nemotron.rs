use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::Path,
    ptr,
    sync::Arc,
};

use libloading::{Library, Symbol};
use regex::Regex;

type OgaResult = c_void;
type OgaConfig = c_void;
type OgaModel = c_void;
type OgaTokenizer = c_void;
type OgaTokenizerStream = c_void;
type OgaGeneratorParams = c_void;
type OgaGenerator = c_void;
type OgaStreamingProcessor = c_void;
type OgaNamedTensors = c_void;

type ResultError = unsafe extern "system" fn(*const OgaResult) -> *const c_char;
type DestroyResult = unsafe extern "system" fn(*mut OgaResult);
type CreateConfig = unsafe extern "system" fn(*const c_char, *mut *mut OgaConfig) -> *mut OgaResult;
type ConfigClearProviders = unsafe extern "system" fn(*mut OgaConfig) -> *mut OgaResult;
type DestroyConfig = unsafe extern "system" fn(*mut OgaConfig);
type CreateModelFromConfig =
    unsafe extern "system" fn(*const OgaConfig, *mut *mut OgaModel) -> *mut OgaResult;
type DestroyModel = unsafe extern "system" fn(*mut OgaModel);
type CreateTokenizer =
    unsafe extern "system" fn(*const OgaModel, *mut *mut OgaTokenizer) -> *mut OgaResult;
type DestroyTokenizer = unsafe extern "system" fn(*mut OgaTokenizer);
type CreateTokenizerStream =
    unsafe extern "system" fn(*const OgaTokenizer, *mut *mut OgaTokenizerStream) -> *mut OgaResult;
type TokenizerStreamDecode =
    unsafe extern "system" fn(*mut OgaTokenizerStream, i32, *mut *const c_char) -> *mut OgaResult;
type DestroyTokenizerStream = unsafe extern "system" fn(*mut OgaTokenizerStream);
type CreateGeneratorParams =
    unsafe extern "system" fn(*const OgaModel, *mut *mut OgaGeneratorParams) -> *mut OgaResult;
type DestroyGeneratorParams = unsafe extern "system" fn(*mut OgaGeneratorParams);
type CreateGenerator = unsafe extern "system" fn(
    *const OgaModel,
    *const OgaGeneratorParams,
    *mut *mut OgaGenerator,
) -> *mut OgaResult;
type DestroyGenerator = unsafe extern "system" fn(*mut OgaGenerator);
type GeneratorSetInputs =
    unsafe extern "system" fn(*mut OgaGenerator, *const OgaNamedTensors) -> *mut OgaResult;
type GeneratorSetRuntimeOption =
    unsafe extern "system" fn(*mut OgaGenerator, *const c_char, *const c_char) -> *mut OgaResult;
type GeneratorIsDone = unsafe extern "system" fn(*mut OgaGenerator) -> bool;
type GeneratorGenerateNextToken = unsafe extern "system" fn(*mut OgaGenerator) -> *mut OgaResult;
type GeneratorGetNextTokens =
    unsafe extern "system" fn(*const OgaGenerator, *mut *const i32, *mut usize) -> *mut OgaResult;
type CreateStreamingProcessor =
    unsafe extern "system" fn(*mut OgaModel, *mut *mut OgaStreamingProcessor) -> *mut OgaResult;
type StreamingProcessorSetOption = unsafe extern "system" fn(
    *mut OgaStreamingProcessor,
    *const c_char,
    *const c_char,
) -> *mut OgaResult;
type StreamingProcessorProcess = unsafe extern "system" fn(
    *mut OgaStreamingProcessor,
    *const f32,
    usize,
    *mut *mut OgaNamedTensors,
) -> *mut OgaResult;
type StreamingProcessorFlush = unsafe extern "system" fn(
    *mut OgaStreamingProcessor,
    *mut *mut OgaNamedTensors,
) -> *mut OgaResult;
type DestroyStreamingProcessor = unsafe extern "system" fn(*mut OgaStreamingProcessor);
type DestroyNamedTensors = unsafe extern "system" fn(*mut OgaNamedTensors);

struct Api {
    _library: Library,
    result_error: ResultError,
    destroy_result: DestroyResult,
    create_config: CreateConfig,
    config_clear_providers: ConfigClearProviders,
    destroy_config: DestroyConfig,
    create_model_from_config: CreateModelFromConfig,
    destroy_model: DestroyModel,
    create_tokenizer: CreateTokenizer,
    destroy_tokenizer: DestroyTokenizer,
    create_tokenizer_stream: CreateTokenizerStream,
    tokenizer_stream_decode: TokenizerStreamDecode,
    destroy_tokenizer_stream: DestroyTokenizerStream,
    create_generator_params: CreateGeneratorParams,
    destroy_generator_params: DestroyGeneratorParams,
    create_generator: CreateGenerator,
    destroy_generator: DestroyGenerator,
    generator_set_inputs: GeneratorSetInputs,
    generator_set_runtime_option: GeneratorSetRuntimeOption,
    generator_is_done: GeneratorIsDone,
    generator_generate_next_token: GeneratorGenerateNextToken,
    generator_get_next_tokens: GeneratorGetNextTokens,
    create_streaming_processor: CreateStreamingProcessor,
    streaming_processor_set_option: StreamingProcessorSetOption,
    streaming_processor_process: StreamingProcessorProcess,
    streaming_processor_flush: StreamingProcessorFlush,
    destroy_streaming_processor: DestroyStreamingProcessor,
    destroy_named_tensors: DestroyNamedTensors,
}

unsafe impl Send for Api {}
unsafe impl Sync for Api {}

impl Api {
    fn load(path: &Path) -> Result<Arc<Self>, String> {
        unsafe {
            #[cfg(windows)]
            let library: Library = {
                use libloading::os::windows::{
                    Library as WindowsLibrary, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
                };
                WindowsLibrary::load_with_flags(
                    path,
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
                )
                .map(Into::into)
            }
            .map_err(|e| {
                format!(
                    "Failed to load ONNX Runtime GenAI at {}: {e}. Click Repair Nemotron in Settings to install its matching private ONNX Runtime.",
                    path.display()
                )
            })?;
            #[cfg(not(windows))]
            let library = Library::new(path).map_err(|e| {
                format!(
                    "Failed to load ONNX Runtime GenAI at {}: {e}",
                    path.display()
                )
            })?;
            macro_rules! function {
                ($name:literal, $kind:ty) => {{
                    let symbol: Symbol<$kind> =
                        library.get(concat!($name, "\0").as_bytes()).map_err(|e| {
                            format!("Missing ONNX Runtime GenAI function {}: {e}", $name)
                        })?;
                    *symbol
                }};
            }
            Ok(Arc::new(Self {
                result_error: function!("OgaResultGetError", ResultError),
                destroy_result: function!("OgaDestroyResult", DestroyResult),
                create_config: function!("OgaCreateConfig", CreateConfig),
                config_clear_providers: function!("OgaConfigClearProviders", ConfigClearProviders),
                destroy_config: function!("OgaDestroyConfig", DestroyConfig),
                create_model_from_config: function!(
                    "OgaCreateModelFromConfig",
                    CreateModelFromConfig
                ),
                destroy_model: function!("OgaDestroyModel", DestroyModel),
                create_tokenizer: function!("OgaCreateTokenizer", CreateTokenizer),
                destroy_tokenizer: function!("OgaDestroyTokenizer", DestroyTokenizer),
                create_tokenizer_stream: function!(
                    "OgaCreateTokenizerStream",
                    CreateTokenizerStream
                ),
                tokenizer_stream_decode: function!(
                    "OgaTokenizerStreamDecode",
                    TokenizerStreamDecode
                ),
                destroy_tokenizer_stream: function!(
                    "OgaDestroyTokenizerStream",
                    DestroyTokenizerStream
                ),
                create_generator_params: function!(
                    "OgaCreateGeneratorParams",
                    CreateGeneratorParams
                ),
                destroy_generator_params: function!(
                    "OgaDestroyGeneratorParams",
                    DestroyGeneratorParams
                ),
                create_generator: function!("OgaCreateGenerator", CreateGenerator),
                destroy_generator: function!("OgaDestroyGenerator", DestroyGenerator),
                generator_set_inputs: function!("OgaGenerator_SetInputs", GeneratorSetInputs),
                generator_set_runtime_option: function!(
                    "OgaGenerator_SetRuntimeOption",
                    GeneratorSetRuntimeOption
                ),
                generator_is_done: function!("OgaGenerator_IsDone", GeneratorIsDone),
                generator_generate_next_token: function!(
                    "OgaGenerator_GenerateNextToken",
                    GeneratorGenerateNextToken
                ),
                generator_get_next_tokens: function!(
                    "OgaGenerator_GetNextTokens",
                    GeneratorGetNextTokens
                ),
                create_streaming_processor: function!(
                    "OgaCreateStreamingProcessor",
                    CreateStreamingProcessor
                ),
                streaming_processor_set_option: function!(
                    "OgaStreamingProcessorSetOption",
                    StreamingProcessorSetOption
                ),
                streaming_processor_process: function!(
                    "OgaStreamingProcessorProcess",
                    StreamingProcessorProcess
                ),
                streaming_processor_flush: function!(
                    "OgaStreamingProcessorFlush",
                    StreamingProcessorFlush
                ),
                destroy_streaming_processor: function!(
                    "OgaDestroyStreamingProcessor",
                    DestroyStreamingProcessor
                ),
                destroy_named_tensors: function!("OgaDestroyNamedTensors", DestroyNamedTensors),
                _library: library,
            }))
        }
    }

    unsafe fn check(&self, result: *mut OgaResult) -> Result<(), String> {
        if result.is_null() {
            return Ok(());
        }
        let error = (self.result_error)(result);
        let message = if error.is_null() {
            "Unknown ONNX Runtime GenAI error".to_string()
        } else {
            CStr::from_ptr(error).to_string_lossy().into_owned()
        };
        (self.destroy_result)(result);
        Err(message)
    }
}

pub struct NemotronModel {
    api: Arc<Api>,
    model: *mut OgaModel,
}

unsafe impl Send for NemotronModel {}

impl NemotronModel {
    pub fn load(runtime_dll: &Path, model_path: &Path) -> Result<Self, String> {
        let api = Api::load(runtime_dll)?;
        let path = CString::new(model_path.to_string_lossy().as_bytes())
            .map_err(|_| "Model path contains a null byte.".to_string())?;
        unsafe {
            let mut config = ptr::null_mut();
            api.check((api.create_config)(path.as_ptr(), &mut config))?;
            let result = (|| {
                api.check((api.config_clear_providers)(config))?;
                let mut model = ptr::null_mut();
                api.check((api.create_model_from_config)(config, &mut model))?;
                Ok(Self {
                    api: Arc::clone(&api),
                    model,
                })
            })();
            (api.destroy_config)(config);
            result
        }
    }

    pub fn start_session(&self, language: &str, use_vad: bool) -> Result<NemotronSession, String> {
        NemotronSession::new(Arc::clone(&self.api), self.model, language, use_vad)
    }
}

impl Drop for NemotronModel {
    fn drop(&mut self) {
        unsafe { (self.api.destroy_model)(self.model) }
    }
}

pub struct NemotronSession {
    api: Arc<Api>,
    processor: *mut OgaStreamingProcessor,
    tokenizer: *mut OgaTokenizer,
    tokenizer_stream: *mut OgaTokenizerStream,
    params: *mut OgaGeneratorParams,
    generator: *mut OgaGenerator,
    buffer: Vec<f32>,
    text: String,
    silent_samples: usize,
}

impl NemotronSession {
    fn new(
        api: Arc<Api>,
        model: *mut OgaModel,
        language: &str,
        enable_vad: bool,
    ) -> Result<Self, String> {
        unsafe {
            let mut processor = ptr::null_mut();
            api.check((api.create_streaming_processor)(model, &mut processor))?;
            let mut tokenizer = ptr::null_mut();
            api.check((api.create_tokenizer)(model, &mut tokenizer))?;
            let mut tokenizer_stream = ptr::null_mut();
            api.check((api.create_tokenizer_stream)(
                tokenizer,
                &mut tokenizer_stream,
            ))?;
            let mut params = ptr::null_mut();
            api.check((api.create_generator_params)(model, &mut params))?;
            let mut generator = ptr::null_mut();
            api.check((api.create_generator)(model, params, &mut generator))?;

            let use_vad = CString::new("use_vad").unwrap();
            let false_value = CString::new(if enable_vad { "true" } else { "false" }).unwrap();
            api.check((api.streaming_processor_set_option)(
                processor,
                use_vad.as_ptr(),
                false_value.as_ptr(),
            ))?;
            let key = CString::new("lang_id").unwrap();
            let value = CString::new(language_id(language).unwrap_or(0).to_string()).unwrap();
            api.check((api.generator_set_runtime_option)(
                generator,
                key.as_ptr(),
                value.as_ptr(),
            ))?;

            Ok(Self {
                api,
                processor,
                tokenizer,
                tokenizer_stream,
                params,
                generator,
                buffer: Vec::new(),
                text: String::new(),
                silent_samples: 0,
            })
        }
    }

    pub fn push(&mut self, audio: &[f32]) -> Result<String, String> {
        self.buffer.extend_from_slice(audio);
        while self.buffer.len() >= 8960 {
            let chunk: Vec<f32> = self.buffer.drain(..8960).collect();
            self.process(&chunk)?;
        }
        Ok(clean_text(&self.text))
    }

    pub fn silent_seconds(&self) -> f32 {
        self.silent_samples as f32 / 16000.0
    }

    pub fn finish(&mut self) -> Result<String, String> {
        if !self.buffer.is_empty() {
            let remaining = std::mem::take(&mut self.buffer);
            self.process(&remaining)?;
        }
        unsafe {
            let mut tensors = ptr::null_mut();
            self.api.check((self.api.streaming_processor_flush)(
                self.processor,
                &mut tensors,
            ))?;
            self.decode_tensors(tensors)?;
        }
        Ok(clean_text(&self.text))
    }

    fn process(&mut self, audio: &[f32]) -> Result<(), String> {
        unsafe {
            let mut tensors = ptr::null_mut();
            self.api.check((self.api.streaming_processor_process)(
                self.processor,
                audio.as_ptr(),
                audio.len(),
                &mut tensors,
            ))?;
            if audio.len() == 8960 {
                if tensors.is_null() {
                    self.silent_samples += audio.len();
                } else {
                    self.silent_samples = 0;
                }
            }
            self.decode_tensors(tensors)
        }
    }

    unsafe fn decode_tensors(&mut self, tensors: *mut OgaNamedTensors) -> Result<(), String> {
        if tensors.is_null() {
            return Ok(());
        }
        let result = (|| {
            self.api
                .check((self.api.generator_set_inputs)(self.generator, tensors))?;
            while !(self.api.generator_is_done)(self.generator) {
                self.api
                    .check((self.api.generator_generate_next_token)(self.generator))?;
                let mut tokens = ptr::null();
                let mut count = 0usize;
                self.api.check((self.api.generator_get_next_tokens)(
                    self.generator,
                    &mut tokens,
                    &mut count,
                ))?;
                if count > 0 {
                    let mut output = ptr::null();
                    self.api.check((self.api.tokenizer_stream_decode)(
                        self.tokenizer_stream,
                        *tokens,
                        &mut output,
                    ))?;
                    if !output.is_null() {
                        self.text
                            .push_str(&CStr::from_ptr(output).to_string_lossy());
                    }
                }
            }
            Ok(())
        })();
        (self.api.destroy_named_tensors)(tensors);
        result
    }
}

impl Drop for NemotronSession {
    fn drop(&mut self) {
        unsafe {
            (self.api.destroy_generator)(self.generator);
            (self.api.destroy_generator_params)(self.params);
            (self.api.destroy_tokenizer_stream)(self.tokenizer_stream);
            (self.api.destroy_tokenizer)(self.tokenizer);
            (self.api.destroy_streaming_processor)(self.processor);
        }
    }
}

fn clean_text(text: &str) -> String {
    let tags = Regex::new(r"\s*<[a-z]{2}(?:-[A-Z]{2})?>\s*").expect("valid regex");
    tags.replace_all(text, " ").trim().to_string()
}

pub fn language_id(language: &str) -> Option<i32> {
    Some(match language {
        "en" | "en-US" => 0,
        "en-GB" => 1,
        "es-ES" => 2,
        "es" | "es-US" => 3,
        "zh-CN" => 4,
        "hi" | "hi-IN" => 6,
        "ar" => 7,
        "fr" | "fr-FR" => 8,
        "de" => 9,
        "ja" => 10,
        "ru" => 11,
        "pt-BR" => 12,
        "pt" | "pt-PT" => 13,
        "ko" => 14,
        "it" => 15,
        "nl" => 16,
        "pl" => 17,
        "tr" => 18,
        "uk" => 19,
        "ro" => 20,
        "el" => 21,
        "cs" => 22,
        "hu" => 23,
        "sv" => 24,
        "da" => 25,
        "fi" => 26,
        "sk" => 28,
        "hr" => 29,
        "bg" => 30,
        "lt" => 31,
        "th" => 32,
        "vi" => 33,
        "et" => 60,
        "lv" => 61,
        "sl" => 62,
        "he" => 64,
        "fr-CA" => 100,
        "auto" => 101,
        "mt" => 102,
        "nb" => 103,
        "nn" => 104,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore = "requires installed Nemotron model and native runtime; run explicitly with TS_STT_TEST_MODEL_DIR and TS_STT_TEST_RUNTIME_DLL"]
    fn loads_installed_model_when_paths_are_provided() {
        let Ok(runtime) = std::env::var("TS_STT_TEST_RUNTIME_DLL") else {
            return;
        };
        let Ok(model) = std::env::var("TS_STT_TEST_MODEL_DIR") else {
            return;
        };
        let loaded = NemotronModel::load(&PathBuf::from(runtime), &PathBuf::from(model));
        assert!(
            loaded.is_ok(),
            "Nemotron model failed to load: {:?}",
            loaded.err()
        );
    }
}
