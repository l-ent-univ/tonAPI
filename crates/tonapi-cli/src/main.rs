//! `tonapi` : outils pour la configuration des universités.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tonapi_config::{
    Registry, establishment_schema, establishment_template, is_valid_id, providers,
    university_schema, university_template,
};

#[derive(Parser)]
#[command(name = "tonapi", version, about = "Outils de tonAPI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Vérifie les fichiers d'universités ; code de sortie 1 si un fichier est refusé.
    Validate {
        #[arg(default_value = "universities")]
        dir: PathBuf,
    },
    /// Écrit le JSON Schema des fichiers de configuration.
    Schema {
        #[arg(long, default_value = "schema")]
        out: PathBuf,
        /// Échoue si les fichiers présents ne sont plus à jour, sans rien écrire.
        #[arg(long)]
        check: bool,
    },
    /// Crée le fichier d'une nouvelle université ou composante à partir d'un modèle commenté.
    New {
        #[command(subcommand)]
        what: NewCommand,
        #[arg(long, global = true, default_value = "universities")]
        dir: PathBuf,
    },
    /// Liste les fournisseurs, service par service, avec leurs paramètres.
    Providers {
        #[arg(long)]
        json: bool,
    },
    /// Affiche la spécification OpenAPI de l'API.
    Openapi {
        /// Fichier de sortie ; sortie standard par défaut.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Génère une clé de session `kid:base64` pour `TONAPI_SESSION_KEYS`.
    Keygen {
        /// Identifiant de la clé (1 à 16 caractères a-z, A-Z, 0-9, `_`).
        #[arg(default_value = "k1")]
        kid: String,
    },
}

#[derive(Subcommand)]
enum NewCommand {
    /// `universities/<id>/university.toml`
    University { id: String },
    /// `universities/<université>/establishments/<id>.toml`
    Establishment { university: String, id: String },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("erreur : {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Validate { dir } => Ok(validate(&dir)),
        Command::Schema { out, check } => schema(&out, check),
        Command::New { what, dir } => new(&what, &dir).map(|()| ExitCode::SUCCESS),
        Command::Providers { json } => {
            print_providers(json)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Openapi { out } => {
            let text = tonapi_server::openapi().to_pretty_json()? + "\n";
            match out {
                Some(path) => fs::write(&path, text)
                    .with_context(|| format!("écriture de {}", path.display()))?,
                None => print!("{text}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Keygen { kid } => {
            println!("{}", tonapi_session::generate_key(&kid)?);
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn validate(dir: &Path) -> ExitCode {
    let (registry, report) = Registry::load(dir);
    for file in &report.accepted {
        println!("✓ {}", file.display());
    }
    for issue in &report.rejected {
        println!("✗ {issue}");
    }
    let establishments: usize = registry.iter().map(|u| u.establishments.len()).sum();
    println!(
        "\n{} université(s), {} composante(s) acceptée(s), {} problème(s)",
        registry.len(),
        establishments,
        report.rejected.len()
    );
    if report.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn schema(out: &Path, check: bool) -> Result<ExitCode> {
    let files = [
        ("university.schema.json", university_schema()),
        ("establishment.schema.json", establishment_schema()),
    ];
    let mut stale = false;
    for (name, value) in files {
        let path = out.join(name);
        let text = serde_json::to_string_pretty(&value)? + "\n";
        if check {
            let current = fs::read_to_string(&path).unwrap_or_default();
            if current != text {
                println!(
                    "✗ {} n'est pas à jour : lancez `tonapi schema`",
                    path.display()
                );
                stale = true;
            }
        } else {
            fs::create_dir_all(out)?;
            fs::write(&path, text).with_context(|| format!("écriture de {}", path.display()))?;
            println!("✓ {}", path.display());
        }
    }
    Ok(if stale {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn new(what: &NewCommand, dir: &Path) -> Result<()> {
    let (path, text) = match what {
        NewCommand::University { id } => {
            check_id(id)?;
            fs::create_dir_all(dir.join(id).join("establishments"))?;
            (
                dir.join(id).join("university.toml"),
                university_template(id),
            )
        }
        NewCommand::Establishment { university, id } => {
            check_id(id)?;
            let univ_dir = dir.join(university);
            if !univ_dir.join("university.toml").is_file() {
                bail!(
                    "{} n'existe pas : créez d'abord l'université avec `tonapi new university {university}`",
                    univ_dir.join("university.toml").display()
                );
            }
            fs::create_dir_all(univ_dir.join("establishments"))?;
            (
                univ_dir.join("establishments").join(format!("{id}.toml")),
                establishment_template(university, id),
            )
        }
    };
    if path.exists() {
        bail!("{} existe déjà", path.display());
    }
    fs::write(&path, text).with_context(|| format!("écriture de {}", path.display()))?;
    println!("✓ {} créé", path.display());
    println!("  Remplacez les « <à remplir> », puis lancez `tonapi validate`.");
    Ok(())
}

fn check_id(id: &str) -> Result<()> {
    if !is_valid_id(id) {
        bail!("identifiant `{id}` invalide : seulement a-z, 0-9 et `-` (ex. `iut-lannion`)");
    }
    Ok(())
}

fn print_providers(json: bool) -> Result<()> {
    let list = providers();
    if json {
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }
    let mut service = String::new();
    for p in list {
        if p.service != service {
            service.clone_from(&p.service);
            println!("\n[services.{service}]");
        }
        println!("  provider = \"{}\"  {}", p.id, p.description);
        if !p.required.is_empty() {
            println!("      obligatoire : {}", p.required.join(", "));
        }
        if !p.optional.is_empty() {
            println!("      facultatif  : {}", p.optional.join(", "));
        }
    }
    Ok(())
}
