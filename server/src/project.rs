use std::{
    collections::{HashMap, HashSet},
    fmt,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

use failure::Fail;
use rbx_tree::RbxValue;

pub static PROJECT_FILENAME: &'static str = "roblox-project.json";

// Serde is silly.
const fn yeah() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum SourceProjectNode {
    Instance {
        #[serde(rename = "$className")]
        class_name: String,

        #[serde(rename = "$properties", default = "HashMap::new")]
        properties: HashMap<String, RbxValue>,

        #[serde(rename = "$ignoreUnknownInstances", default = "yeah")]
        ignore_unknown_instances: bool,

        #[serde(flatten)]
        children: HashMap<String, SourceProjectNode>,
    },
    SyncPoint {
        #[serde(rename = "$path")]
        path: String,
    }
}

impl SourceProjectNode {
    pub fn into_project_node(self, project_file_location: &Path) -> ProjectNode {
        match self {
            SourceProjectNode::Instance { class_name, mut children, properties, ignore_unknown_instances } => {
                let mut new_children = HashMap::new();

                for (node_name, node) in children.drain() {
                    new_children.insert(node_name, node.into_project_node(project_file_location));
                }

                ProjectNode::Instance(InstanceProjectNode {
                    class_name,
                    children: new_children,
                    properties,
                    metadata: InstanceProjectNodeMetadata {
                        ignore_unknown_instances,
                    },
                })
            },
            SourceProjectNode::SyncPoint { path: source_path } => {
                let path = if Path::new(&source_path).is_absolute() {
                    PathBuf::from(source_path)
                } else {
                    let project_folder_location = project_file_location.parent().unwrap();
                    project_folder_location.join(source_path)
                };

                ProjectNode::SyncPoint(SyncPointProjectNode {
                    path,
                })
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceProject {
    name: String,
    tree: SourceProjectNode,
    serve_port: Option<u16>,
    serve_place_ids: Option<HashSet<u64>>,
    serve_place_id: Option<u64>,
}

impl SourceProject {
    pub fn into_project(self, project_file_location: &Path) -> Project {
        let tree = self.tree.into_project_node(project_file_location);

        Project {
            name: self.name,
            tree,
            serve_port: self.serve_port,
            serve_place_ids: self.serve_place_ids,
            file_location: PathBuf::from(project_file_location),
        }
    }
}

#[derive(Debug, Fail)]
pub enum ProjectLoadExactError {
    #[fail(display = "IO error: {}", _0)]
    IoError(#[fail(cause)] io::Error),

    #[fail(display = "JSON error: {}", _0)]
    JsonError(#[fail(cause)] serde_json::Error),
}

#[derive(Debug, Fail)]
pub enum ProjectLoadFuzzyError {
    #[fail(display = "Project not found")]
    NotFound,

    #[fail(display = "IO error: {}", _0)]
    IoError(#[fail(cause)] io::Error),

    #[fail(display = "JSON error: {}", _0)]
    JsonError(#[fail(cause)] serde_json::Error),
}

impl From<ProjectLoadExactError> for ProjectLoadFuzzyError {
    fn from(error: ProjectLoadExactError) -> ProjectLoadFuzzyError {
        match error {
            ProjectLoadExactError::IoError(inner) => ProjectLoadFuzzyError::IoError(inner),
            ProjectLoadExactError::JsonError(inner) => ProjectLoadFuzzyError::JsonError(inner),
        }
    }
}

#[derive(Debug, Fail)]
pub enum ProjectInitError {
    AlreadyExists(PathBuf),
    IoError(#[fail(cause)] io::Error),
    SaveError(#[fail(cause)] ProjectSaveError),
}

impl fmt::Display for ProjectInitError {
    fn fmt(&self, output: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ProjectInitError::AlreadyExists(path) => write!(output, "Path {} already exists", path.display()),
            ProjectInitError::IoError(inner) => write!(output, "IO error: {}", inner),
            ProjectInitError::SaveError(inner) => write!(output, "{}", inner),
        }
    }
}

#[derive(Debug, Fail)]
pub enum ProjectSaveError {
    #[fail(display = "JSON error: {}", _0)]
    JsonError(#[fail(cause)] serde_json::Error),

    #[fail(display = "IO error: {}", _0)]
    IoError(#[fail(cause)] io::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceProjectNodeMetadata {
    pub ignore_unknown_instances: bool,
}

impl Default for InstanceProjectNodeMetadata {
    fn default() -> InstanceProjectNodeMetadata {
        InstanceProjectNodeMetadata {
            ignore_unknown_instances: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProjectNode {
    Instance(InstanceProjectNode),
    SyncPoint(SyncPointProjectNode),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceProjectNode {
    pub class_name: String,
    pub children: HashMap<String, ProjectNode>,
    pub properties: HashMap<String, RbxValue>,
    pub metadata: InstanceProjectNodeMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPointProjectNode {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub tree: ProjectNode,
    pub serve_port: Option<u16>,
    pub serve_place_ids: Option<HashSet<u64>>,
    pub file_location: PathBuf,
}

impl Project {
    pub fn init_place(project_fuzzy_location: &Path) -> Result<PathBuf, ProjectInitError> {
        let is_exact = project_fuzzy_location.extension().is_some();

        let project_name = if is_exact {
            project_fuzzy_location.parent().unwrap().file_name().unwrap().to_str().unwrap()
        } else {
            project_fuzzy_location.file_name().unwrap().to_str().unwrap()
        };

        // TODO: Add children for src folder, potentially client, server, and
        // common?

        let replicated_storage_children = HashMap::new();

        let replicated_storage = ProjectNode::Instance(InstanceProjectNode {
            class_name: "ReplicatedStorage".to_string(),
            children: replicated_storage_children,
            properties: HashMap::new(),
            metadata: Default::default(),
        });

        let mut root_children = HashMap::new();
        root_children.insert("ReplicatedStorage".to_string(), replicated_storage);

        let tree = ProjectNode::Instance(InstanceProjectNode {
            class_name: "DataModel".to_string(),
            children: root_children,
            properties: HashMap::new(),
            metadata: Default::default(),
        });

        let project = Project {
            name: project_name.to_string(),
            tree,
            serve_port: None,
            serve_place_ids: None,
            file_location: project_fuzzy_location.to_path_buf(),
        };

        Project::init_internal(project_fuzzy_location, &project)
    }

    pub fn init_model(_project_fuzzy_location: &Path) -> Result<PathBuf, ProjectInitError> {
        unimplemented!();
    }

    fn init_internal(project_fuzzy_location: &Path, project: &Project) -> Result<PathBuf, ProjectInitError> {
        let is_exact = project_fuzzy_location.extension().is_some();

        let project_location = if is_exact {
            project_fuzzy_location.to_path_buf()
        } else {
            project_fuzzy_location.join(PROJECT_FILENAME)
        };

        match fs::metadata(&project_location) {
            Err(error) => match error.kind() {
                io::ErrorKind::NotFound => {},
                _ => return Err(ProjectInitError::IoError(error)),
            },
            Ok(_) => return Err(ProjectInitError::AlreadyExists(project_location)),
        }

        project.save(&project_location)
            .map_err(ProjectInitError::SaveError)?;

        Ok(project_location)
    }

    pub fn locate(start_location: &Path) -> Option<PathBuf> {
        // TODO: Check for specific error kinds, convert 'not found' to Result.
        let location_metadata = fs::metadata(start_location).ok()?;

        // If this is a file, we should assume it's the config we want
        if location_metadata.is_file() {
            return Some(start_location.to_path_buf());
        } else if location_metadata.is_dir() {
            let with_file = start_location.join(PROJECT_FILENAME);

            if let Ok(with_file_metadata) = fs::metadata(&with_file) {
                if with_file_metadata.is_file() {
                    return Some(with_file);
                } else {
                    return None;
                }
            }
        }

        match start_location.parent() {
            Some(parent_location) => Self::locate(parent_location),
            None => None,
        }
    }

    pub fn load_fuzzy(fuzzy_project_location: &Path) -> Result<Project, ProjectLoadFuzzyError> {
        let project_path = Self::locate(fuzzy_project_location)
            .ok_or(ProjectLoadFuzzyError::NotFound)?;

        Self::load_exact(&project_path).map_err(From::from)
    }

    pub fn load_exact(project_file_location: &Path) -> Result<Project, ProjectLoadExactError> {
        let contents = fs::read_to_string(project_file_location)
            .map_err(ProjectLoadExactError::IoError)?;

        let parsed: SourceProject = serde_json::from_str(&contents)
            .map_err(ProjectLoadExactError::JsonError)?;

        Ok(parsed.into_project(project_file_location))
    }

    pub fn save(&self, path: &Path) -> Result<(), ProjectSaveError> {
        let source_project = self.to_source_project();
        let mut file = File::create(path)
            .map_err(ProjectSaveError::IoError)?;

        serde_json::to_writer_pretty(&mut file, &source_project)
            .map_err(ProjectSaveError::JsonError)?;

        Ok(())
    }

    fn to_source_project(&self) -> SourceProject {
        unimplemented!();
    }
}