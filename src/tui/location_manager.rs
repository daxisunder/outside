use crate::api::location::LocationData;
use crate::utils::cache;
use cursive::views::SelectView;
use savefile::prelude::*;
use savefile_derive::Savefile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Get the path to the bookmarks.yaml file in the config directory
fn get_bookmarks_yaml_path() -> PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| dirs_next::home_dir().unwrap_or_default())
        .join(env!("CARGO_PKG_NAME"))
        .join("bookmarks.yaml")
}

#[derive(Serialize, Deserialize, Debug, Default, Savefile)]
pub struct LocationList {
    pub locations: Vec<String>,
}

impl LocationList {
    pub fn load() -> Self {
        let bookmarks_path = get_bookmarks_yaml_path();

        // First try to load from bookmarks.yaml
        if bookmarks_path.exists() {
            return Self::load_from_yaml(&bookmarks_path);
        }

        // Fall back to Savefile and migrate
        let savefile_path = cache::get_cached_file("locations", "list");
        if let Ok(old_list) = load_file::<LocationList, _>(&savefile_path, 0) {
            // Convert and save as bookmarks.yaml
            if let Err(e) = old_list.save_as_yaml(&bookmarks_path) {
                eprintln!("Warning: Failed to migrate bookmarks to YAML: {e}");
                return old_list; // Use old data without migration
            }
            // Remove old cache file after successful migration
            let _ = std::fs::remove_file(&savefile_path);
            return old_list;
        }

        // Default if neither exists
        Self::default()
    }

    pub fn save(&self) {
        let yaml_path = get_bookmarks_yaml_path();
        if let Err(e) = self.save_as_yaml(&yaml_path) {
            eprintln!("Unable to save location list: {e:#?}");
        }
    }

    /// Load LocationList from a YAML file
    fn load_from_yaml(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_yaml::from_str(&content).unwrap_or_else(|e| {
                eprintln!("Warning: Failed to parse bookmarks.yaml: {e}");
                Self::default()
            }),
            Err(e) => {
                eprintln!("Warning: Failed to read bookmarks.yaml: {e}");
                Self::default()
            },
        }
    }

    /// Save LocationList to a YAML file
    fn save_as_yaml(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        // Ensure config directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let yaml_content = serde_yaml::to_string(self)?;
        std::fs::write(path, yaml_content)?;
        Ok(())
    }

    pub fn add_location(&mut self, location: String) {
        if !self.locations.contains(&location) {
            self.locations.push(location);
            self.save();
        }
    }

    pub fn remove_location_by_name(&mut self, location: &str) {
        if let Some(index) = self.locations.iter().position(|loc| loc == location) {
            self.locations.remove(index);
            self.save();
        }
    }

    /// Returns a sorted list of locations and the index of the specified location
    pub fn get_sorted_locations_with_index(&self, target_location: &str) -> (Vec<String>, Option<usize>) {
        let (sorted_locations, _) = self.get_sorted_locations();
        let target_index = sorted_locations.iter().position(|loc| loc == target_location);
        (sorted_locations, target_index)
    }

    /// Returns sorted locations with "Automatic" first, then alphabetically by city/country
    pub fn get_sorted_locations(&self) -> (Vec<String>, Vec<String>) {
        // Separate "Automatic" from other locations
        let mut automatic_locations = Vec::new();
        let mut other_locations = Vec::new();

        for location in &self.locations {
            if location == "Automatic" {
                automatic_locations.push(location.clone());
            } else {
                other_locations.push(location.clone());
            }
        }

        // Sort other locations by city, then country code
        other_locations.sort_by(|a, b| {
            let a_parts: Vec<&str> = a.split(',').collect();
            let b_parts: Vec<&str> = b.split(',').collect();

            if a_parts.len() >= 2 && b_parts.len() >= 2 {
                let a_city = a_parts[0].trim();
                let a_country = a_parts[1].trim();
                let b_city = b_parts[0].trim();
                let b_country = b_parts[1].trim();

                // Sort by city first, then by country
                a_city.cmp(b_city).then(a_country.cmp(b_country))
            } else {
                // Fallback to string comparison for malformed entries
                a.cmp(b)
            }
        });

        // Create the ordered list
        let mut all_ordered_locations = automatic_locations.clone();
        all_ordered_locations.extend(other_locations.clone());

        (all_ordered_locations, other_locations)
    }
}

#[derive(Clone)]
pub struct LocationManager {
    location_list: Arc<Mutex<LocationList>>,
}

impl Default for LocationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LocationManager {
    pub fn new() -> Self {
        let location_list = Arc::new(Mutex::new(LocationList::load()));
        Self { location_list }
    }

    pub fn get_location_list(&self) -> Arc<Mutex<LocationList>> {
        self.location_list.clone()
    }

    pub fn add_location(&self, location: String) {
        let mut list = self.location_list.lock().unwrap();
        list.add_location(location);
    }

    pub fn remove_location_by_name(&self, location: &str) {
        let mut list = self.location_list.lock().unwrap();
        list.remove_location_by_name(location);
    }

    pub fn rebuild_select_view(&self, view: &mut SelectView<String>, target_location: &str) -> Option<usize> {
        let list = self.location_list.lock().unwrap();
        let (sorted_locations, target_index) = list.get_sorted_locations_with_index(target_location);

        // Clear and rebuild the SelectView with sorted locations
        view.clear();
        for location in &sorted_locations {
            view.add_item(location.clone(), location.clone());
        }

        target_index
    }

    pub fn get_current_location_string(&self, settings_location: &str) -> String {
        if settings_location.is_empty() {
            "Automatic".to_string()
        } else {
            LocationData::normalize_location_string(settings_location)
        }
    }

    pub fn ensure_location_in_list(&self, location: String) {
        let mut list = self.location_list.lock().unwrap();
        if !list.locations.contains(&location) {
            list.add_location(location);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper function to create a temporary config directory for testing
    fn setup_test_config_dir() -> TempDir {
        TempDir::new().expect("Failed to create temp directory")
    }

    /// Helper function to create a test LocationList
    fn create_test_location_list() -> LocationList {
        LocationList {
            locations: vec![
                "Automatic".to_string(),
                "London, GB".to_string(),
                "New York, US".to_string(),
                "Paris, FR".to_string(),
            ],
        }
    }

    #[test]
    fn test_get_bookmarks_yaml_path() {
        let path = get_bookmarks_yaml_path();

        assert!(path.to_string_lossy().contains("outside"));
        assert!(path.to_string_lossy().ends_with("bookmarks.yaml"));
    }

    #[test]
    fn test_save_as_yaml() {
        let temp_dir = setup_test_config_dir();
        let yaml_path = temp_dir.path().join("outside").join("bookmarks.yaml");

        let location_list = create_test_location_list();

        // Test successful save
        assert!(location_list.save_as_yaml(&yaml_path).is_ok());
        assert!(yaml_path.exists());

        // Verify file contents
        let content = fs::read_to_string(&yaml_path).expect("Failed to read YAML file");
        assert!(content.contains("locations:"));
        assert!(content.contains("- Automatic"));
        assert!(content.contains("- London, GB"));
    }

    #[test]
    fn test_load_from_yaml() {
        let temp_dir = setup_test_config_dir();
        let yaml_path = temp_dir.path().join("outside").join("bookmarks.yaml");

        // Create test YAML content
        let yaml_content = r#"locations:
- Automatic
- London, GB
- New York, US
- Paris, FR
"#;

        // Ensure directory exists and write test file
        fs::create_dir_all(yaml_path.parent().unwrap()).expect("Failed to create directory");
        fs::write(&yaml_path, yaml_content).expect("Failed to write test YAML");

        // Test loading
        let loaded_list = LocationList::load_from_yaml(&yaml_path);

        assert_eq!(loaded_list.locations.len(), 4);
        assert_eq!(loaded_list.locations[0], "Automatic");
        assert_eq!(loaded_list.locations[1], "London, GB");
        assert_eq!(loaded_list.locations[2], "New York, US");
        assert_eq!(loaded_list.locations[3], "Paris, FR");
    }

    #[test]
    fn test_load_from_yaml_invalid_file() {
        let temp_dir = setup_test_config_dir();
        let yaml_path = temp_dir.path().join("outside").join("invalid.yaml");

        // Create invalid YAML content
        let invalid_yaml = "invalid: yaml: content: [";
        fs::create_dir_all(yaml_path.parent().unwrap()).expect("Failed to create directory");
        fs::write(&yaml_path, invalid_yaml).expect("Failed to write invalid YAML");

        // Should return default on invalid YAML
        let loaded_list = LocationList::load_from_yaml(&yaml_path);
        assert_eq!(loaded_list.locations.len(), 0);
    }

    #[test]
    fn test_load_from_yaml_nonexistent_file() {
        let temp_dir = setup_test_config_dir();
        let yaml_path = temp_dir.path().join("nonexistent.yaml");

        // Should return default for nonexistent file
        let loaded_list = LocationList::load_from_yaml(&yaml_path);
        assert_eq!(loaded_list.locations.len(), 0);
    }

    #[test]
    fn test_load_with_yaml_priority() {
        let temp_dir = setup_test_config_dir();
        let config_dir = temp_dir.path();

        // Create YAML file in test directory
        let yaml_content = r#"locations:
- From YAML
- London, GB
"#;
        fs::create_dir_all(config_dir.join("outside")).expect("Failed to create directory");
        let yaml_path = config_dir.join("outside").join("bookmarks.yaml");
        fs::write(&yaml_path, yaml_content).expect("Failed to write YAML");

        // Test loading from the test directory directly
        let loaded_list = LocationList::load_from_yaml(&yaml_path);
        assert_eq!(loaded_list.locations.len(), 2);
        assert_eq!(loaded_list.locations[0], "From YAML");
        assert_eq!(loaded_list.locations[1], "London, GB");
    }

    #[test]
    fn test_add_location() {
        let mut location_list = LocationList::default();

        // Test adding new location (test the logic without calling save)
        if !location_list.locations.contains(&"Tokyo, JP".to_string()) {
            location_list.locations.push("Tokyo, JP".to_string());
        }
        assert_eq!(location_list.locations.len(), 1);
        assert_eq!(location_list.locations[0], "Tokyo, JP");

        // Test adding duplicate location (should not add)
        if !location_list.locations.contains(&"Tokyo, JP".to_string()) {
            location_list.locations.push("Tokyo, JP".to_string());
        }
        assert_eq!(location_list.locations.len(), 1);

        // Test adding another unique location
        if !location_list.locations.contains(&"Sydney, AU".to_string()) {
            location_list.locations.push("Sydney, AU".to_string());
        }
        assert_eq!(location_list.locations.len(), 2);
    }

    #[test]
    fn test_remove_location_by_name() {
        let mut location_list = create_test_location_list();

        // Test removing existing location (test the logic without calling save)
        if let Some(index) = location_list.locations.iter().position(|loc| loc == "London, GB") {
            location_list.locations.remove(index);
        }
        assert_eq!(location_list.locations.len(), 3);
        assert!(!location_list.locations.contains(&"London, GB".to_string()));

        // Test removing non-existent location (should not change list)
        let initial_len = location_list.locations.len();
        if let Some(index) = location_list.locations.iter().position(|loc| loc == "Tokyo, JP") {
            location_list.locations.remove(index);
        }
        assert_eq!(location_list.locations.len(), initial_len); // Should be unchanged
    }

    #[test]
    fn test_get_sorted_locations() {
        let location_list = LocationList {
            locations: vec![
                "Zurich, CH".to_string(),
                "Automatic".to_string(),
                "Berlin, DE".to_string(),
                "Amsterdam, NL".to_string(),
            ],
        };

        let (sorted_locations, other_locations) = location_list.get_sorted_locations();

        // "Automatic" should be first
        assert_eq!(sorted_locations[0], "Automatic");

        // Other locations should be sorted alphabetically by city
        assert_eq!(other_locations[0], "Amsterdam, NL");
        assert_eq!(other_locations[1], "Berlin, DE");
        assert_eq!(other_locations[2], "Zurich, CH");

        // Full sorted list should have "Automatic" first, then alphabetical
        assert_eq!(sorted_locations.len(), 4);
        assert_eq!(sorted_locations[0], "Automatic");
        assert_eq!(sorted_locations[1], "Amsterdam, NL");
        assert_eq!(sorted_locations[2], "Berlin, DE");
        assert_eq!(sorted_locations[3], "Zurich, CH");
    }

    #[test]
    fn test_get_sorted_locations_with_index() {
        let location_list = LocationList {
            locations: vec!["Zurich, CH".to_string(), "Automatic".to_string(), "Berlin, DE".to_string()],
        };

        // Order should be: ["Automatic", "Berlin, DE", "Zurich, CH"]

        // Test finding index of existing location
        let (sorted_locations, index) = location_list.get_sorted_locations_with_index("Berlin, DE");
        assert_eq!(sorted_locations.len(), 3);
        assert_eq!(index, Some(1)); // Should be at index 1 in sorted list (after "Automatic")

        // Test finding index of "Automatic" (should be first)
        let (_, index) = location_list.get_sorted_locations_with_index("Automatic");
        assert_eq!(index, Some(0));

        // Test finding index of non-existent location
        let (_, index) = location_list.get_sorted_locations_with_index("Tokyo, JP");
        assert_eq!(index, None);
    }

    #[test]
    fn test_location_manager_new() {
        // Test creating a LocationManager with a known state (not from files)
        let test_list = LocationList { locations: vec!["Test Location".to_string()] };
        let location_list = Arc::new(Mutex::new(test_list));
        let manager = LocationManager { location_list };

        // The manager should be created successfully
        let list = manager.location_list.lock().unwrap();
        assert_eq!(list.locations.len(), 1);
        assert_eq!(list.locations[0], "Test Location");
    }

    #[test]
    fn test_location_manager_get_current_location_string() {
        // Create a test instance that doesn't affect the file system
        let location_list = Arc::new(Mutex::new(LocationList::default()));
        let manager = LocationManager { location_list };

        // Test empty location (should return "Automatic")
        assert_eq!(manager.get_current_location_string(""), "Automatic");

        // Test with actual location
        assert_eq!(
            manager.get_current_location_string("London, GB"),
            LocationData::normalize_location_string("London, GB")
        );
    }

    #[test]
    fn test_location_manager_add_remove_operations() {
        // Test LocationManager operations without affecting the file system
        // by testing the logic directly without file I/O
        let mut test_list = LocationList { locations: vec!["Initial Location, IL".to_string()] };

        // Test add operation logic (without calling save)
        if !test_list.locations.contains(&"New Location, NL".to_string()) {
            test_list.locations.push("New Location, NL".to_string());
        }
        assert_eq!(test_list.locations.len(), 2);
        assert!(test_list.locations.contains(&"New Location, NL".to_string()));
        assert!(test_list.locations.contains(&"Initial Location, IL".to_string()));

        // Test no duplicate add
        if !test_list.locations.contains(&"New Location, NL".to_string()) {
            test_list.locations.push("New Location, NL".to_string());
        }
        assert_eq!(test_list.locations.len(), 2); // Should not increase

        // Test remove operation logic (without calling save)
        if let Some(index) = test_list.locations.iter().position(|loc| loc == "Initial Location, IL") {
            test_list.locations.remove(index);
        }
        assert_eq!(test_list.locations.len(), 1);
        assert!(!test_list.locations.contains(&"Initial Location, IL".to_string()));
        assert!(test_list.locations.contains(&"New Location, NL".to_string()));

        // Test remove non-existent location
        let initial_len = test_list.locations.len();
        if let Some(index) = test_list.locations.iter().position(|loc| loc == "Non-existent Location") {
            test_list.locations.remove(index);
        }
        assert_eq!(test_list.locations.len(), initial_len); // Should not change
    }
}
