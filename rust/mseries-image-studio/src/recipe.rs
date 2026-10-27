use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Recipe {
    pub schema_version: u32,
    pub created_at_unix: u64,
    pub prompt: String,
    pub negative_prompt: String,
    pub intent: String,
    pub workflow: String,
    pub model_id: String,
    pub lora_id: String,
    pub lora_weight: f64,
    pub width: u32,
    pub height: u32,
    pub batch_size: u32,
    pub seed: i64,
    pub tags: Vec<String>,
    pub engine_id: String,
    pub image_path: String,
    pub app_version: String,
    pub elapsed_seconds: Option<f64>,
    pub decoder_seconds: Option<f64>,
    pub sampling_seconds: Option<f64>,
    pub post_sampling_seconds: Option<f64>,
    pub preview_interval: u32,
    pub peak_memory_mib: Option<u64>,
    pub settings_note: String,
    pub steps: u32,
    pub cfg_scale: f64,
    pub sampler: String,
    pub scheduler: String,
    pub vae_path: String,
    pub vae_on_cpu: bool,
    pub reference_image: String,
    pub reference_strength: f64,
    pub vae_tile_size: u32,
    pub cpu_threads: u32,
    pub params_backend: String,
    pub max_vram_gib: f64,
    pub model_receipt: Option<serde_json::Value>,
    pub engine_receipt: Option<serde_json::Value>,
    pub preview_count: u32,
    pub preview_method: String,
    pub time_to_first_preview_seconds: Option<f64>,
    pub minimum_available_mib: Option<u64>,
    pub sweep_id: String,
    pub sweep_variant: String,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            schema_version: 1,
            created_at_unix: now_unix(),
            prompt: String::new(),
            negative_prompt: String::new(),
            intent: "Portrait".into(),
            workflow: "Freeform".into(),
            model_id: String::new(),
            lora_id: String::new(),
            lora_weight: 1.0,
            width: 512,
            height: 512,
            batch_size: 1,
            seed: -1,
            tags: vec![],
            engine_id: "unconfigured".into(),
            image_path: String::new(),
            elapsed_seconds: None,
            decoder_seconds: None,
            sampling_seconds: None,
            post_sampling_seconds: None,
            preview_interval: 3,
            peak_memory_mib: None,
            settings_note: String::new(),
            steps: 20,
            cfg_scale: 7.0,
            sampler: "euler".into(),
            scheduler: "discrete".into(),
            vae_path: String::new(),
            vae_on_cpu: true,
            reference_image: String::new(),
            reference_strength: 0.8,
            vae_tile_size: 256,
            cpu_threads: 4,
            params_backend: "disk".into(),
            max_vram_gib: 3.0,
            model_receipt: None,
            engine_receipt: None,
            preview_count: 0,
            preview_method: "proj".into(),
            time_to_first_preview_seconds: None,
            minimum_available_mib: None,
            sweep_id: String::new(),
            sweep_variant: String::new(),
            app_version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RecipeStore {
    directory: PathBuf,
}

impl RecipeStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn default_store() -> Self {
        Self::new(
            gtk::glib::user_data_dir()
                .join("mseries-image-studio")
                .join("recipes"),
        )
    }

    pub fn save(&self, recipe: &Recipe) -> std::io::Result<PathBuf> {
        fs::create_dir_all(&self.directory)?;
        let name = format!("recipe-{}-{}.json", now_unix_nanos(), std::process::id());
        let path = self.directory.join(name);
        write_private(
            &path,
            &serde_json::to_vec_pretty(recipe).expect("recipe serialization"),
        )?;
        Ok(path)
    }

    pub fn load_recent(&self, limit: usize) -> std::io::Result<Vec<Recipe>> {
        if !self.directory.is_dir() {
            return Ok(vec![]);
        }
        let mut paths = fs::read_dir(&self.directory)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect::<Vec<_>>();
        paths.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        let mut recipes = Vec::new();
        for path in paths.into_iter().take(limit) {
            match fs::read(&path).and_then(|bytes| {
                serde_json::from_slice::<Recipe>(&bytes).map_err(std::io::Error::other)
            }) {
                Ok(recipe) => recipes.push(recipe),
                Err(error) => eprintln!("Skipping unreadable recipe {}: {error}", path.display()),
            }
        }
        Ok(recipes)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

pub fn save_png(png: &[u8], recipe: &mut Recipe, directory: &Path) -> std::io::Result<PathBuf> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "image engine did not return PNG data",
        ));
    }
    fs::create_dir_all(directory)?;
    let base = format!("image-{}-{}", now_unix_nanos(), std::process::id());
    let image = directory.join(format!("{base}.png"));
    let metadata = directory.join(format!("{base}.json"));
    write_private(&image, png)?;
    recipe.image_path = image.to_string_lossy().into_owned();
    if let Err(error) = write_private(
        &metadata,
        &serde_json::to_vec_pretty(recipe).expect("recipe serialization"),
    ) {
        let _ = fs::remove_file(&image);
        recipe.image_path.clear();
        return Err(error);
    }
    Ok(image)
}

pub fn load_outputs(directory: &Path, limit: usize) -> std::io::Result<Vec<Recipe>> {
    let mut recipes = RecipeStore::new(directory).load_recent(limit)?;
    recipes
        .retain(|recipe| !recipe.image_path.is_empty() && Path::new(&recipe.image_path).is_file());
    Ok(recipes)
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mseries-image-studio-{name}-{}-{}",
            std::process::id(),
            now_unix_nanos()
        ))
    }

    #[test]
    fn recipe_round_trip_persists_generation_settings_and_tags() {
        let directory = scratch("recipes");
        let store = RecipeStore::new(&directory);
        let mut recipe = Recipe::default();
        recipe.prompt = "Adult streamer, soft daylight".into();
        recipe.negative_prompt = "watermark, text".into();
        recipe.intent = "Streamer scene".into();
        recipe.workflow = "Prompt type study".into();
        recipe.model_id = "test-model".into();
        recipe.lora_id = "test-lora".into();
        recipe.lora_weight = 0.8;
        recipe.width = 768;
        recipe.height = 768;
        recipe.batch_size = 3;
        recipe.seed = 424242;
        recipe.steps = 16;
        recipe.cfg_scale = 5.5;
        recipe.sampler = "euler_a".into();
        recipe.elapsed_seconds = Some(12.5);
        recipe.peak_memory_mib = Some(1024);
        recipe.settings_note = "Measured local test".into();
        recipe.tags = vec!["portrait".into(), "streamer".into()];
        let path = store.save(&recipe).unwrap();
        let loaded = store.load_recent(10).unwrap();
        assert_eq!(loaded, vec![recipe]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn image_store_requires_png_and_writes_metadata() {
        let directory = scratch("images");
        let mut recipe = Recipe::default();
        recipe.prompt = "Adult streamer".into();
        assert!(save_png(b"nope", &mut recipe.clone(), &directory).is_err());
        let path = save_png(b"\x89PNG\r\n\x1a\nfixture", &mut recipe, &directory).unwrap();
        assert!(path.is_file());
        assert_eq!(load_outputs(&directory, 10).unwrap().len(), 1);
        let _ = fs::remove_dir_all(directory);
    }
}
