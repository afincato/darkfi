use std::process::exit;

use clap::{Parser, Subcommand};
use log::{error, info};
use simplelog::{ColorChoice, TermLogger, TerminalMode};
use url::Url;

use darkfi::{
    rpc::client::RpcClient,
    util::cli::{get_log_config, get_log_level},
    Result,
};

mod drawdown;
mod filter;
mod primitives;
mod rpc;
mod util;
mod view;

use drawdown::{drawdown, to_naivedate};
use filter::{apply_filter, get_ids, no_filter_warn};
use primitives::{task_from_cli, State, TaskEvent};
use util::{desc_in_editor, due_as_timestamp};
use view::{comments_as_string, print_task_info, print_task_list};

const DEFAULT_PATH: &str = "~/tau_exported_tasks";

#[derive(Parser)]
#[clap(name = "tau", version)]
#[clap(subcommand_precedence_over_arg = true)]
struct Args {
    #[clap(short, parse(from_occurrences))]
    /// Increase verbosity (-vvv supported)
    verbose: u8,

    #[clap(short, long, default_value = "tcp://127.0.0.1:23330")]
    /// taud JSON-RPC endpoint
    endpoint: Url,

    /// Search filters (zero or more)
    filters: Vec<String>,

    #[clap(subcommand)]
    command: Option<TauSubcommand>,
}

#[derive(Subcommand)]
enum TauSubcommand {
    /// Add a new task.
    ///
    /// Quick start:
    ///   Adding a new task named "New task":
    ///     tau add New task
    ///   New task with description:
    ///     tau add Add more info to tau desc:"some awesome description"
    ///   New task with project and assignee:
    ///     tau add Third task project:p2p assign:rusty
    ///   Add a task with due date September 12th and rank of 4.6:
    ///     tau add Task no. Four due:1209 rank:4.6
    ///
    /// Notice that if the command does not have "desc" key it will open
    /// an Editor so you can write the description there.
    ///
    /// Also note that "project" and "assign" keys can have multiple
    /// comma-separated values.
    ///
    /// All keys example:
    ///     tau add Improve CLI desc:"Description here" project:tau,ircd assign:dave,rusty due:0210 rank:2.2
    ///
    #[clap(verbatim_doc_comment)]
    Add {
        /// Pairs of key:value (e.g. desc:description assign:dark).
        values: Vec<String>,
    },

    /// Modify/Edit an existing task.
    Modify {
        #[clap(allow_hyphen_values = true)]
        /// Values (e.g. project:blockchain).
        values: Vec<String>,
    },

    /// List tasks.
    List,

    /// Start task(s).
    Start,

    /// Open task(s).
    Open,

    /// Pause task(s).
    Pause,

    /// Stop task(s).
    Stop,

    /// Set or Get comment for task(s).
    Comment {
        /// Set comment content if provided (Get comments otherwise).
        content: Vec<String>,
    },

    /// Get all data about selected task(s).
    Info,

    /// Switch workspace.
    Switch {
        /// Tau workspace.
        workspace: String,
    },

    /// Import tasks from a specified directory.
    Import {
        /// The parent directory from where you want to import tasks.
        path: Option<String>,
    },

    /// Export tasks to a specified directory.
    Export {
        /// The parent directory to where you want to export tasks.
        path: Option<String>,
    },

    /// Log drawdown.
    Log {
        /// The month in which we want to draw a heatmap (e.g. 0822 for August 2022).
        month: Option<String>,
        /// The person of which we want to draw a heatmap
        /// (if not provided we list all assignees).
        assignee: Option<String>,
    },
}

pub struct Tau {
    pub rpc_client: RpcClient,
}

#[async_std::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let log_level = get_log_level(args.verbose.into());
    let log_config = get_log_config();
    TermLogger::init(log_level, log_config, TerminalMode::Mixed, ColorChoice::Auto)?;

    let rpc_client = RpcClient::new(args.endpoint).await?;
    let tau = Tau { rpc_client };

    let mut filters = args.filters.clone();

    // If IDs are provided in filter we use them to get the tasks from the daemon
    // then remove IDs from filter so we can do apply_filter() normally.
    // If not provided we use get_ids() to get them from the daemon.
    let ids = get_ids(&mut filters)?;
    let task_ids = if ids.is_empty() { tau.get_ids().await? } else { ids };

    let mut tasks =
        if filters.contains(&"state:stop".to_string()) || filters.contains(&"all".to_string()) {
            tau.get_stop_tasks(None).await?
        } else {
            vec![]
        };
    for id in task_ids {
        tasks.push(tau.get_task_by_id(id).await?);
    }

    for filter in filters {
        apply_filter(&mut tasks, &filter);
    }

    // Parse subcommands
    match args.command {
        Some(sc) => match sc {
            TauSubcommand::Add { values } => {
                let mut task = task_from_cli(values)?;
                if task.title.is_empty() {
                    error!("Please provide a title for the task.");
                    exit(1);
                };

                if task.desc.is_none() {
                    task.desc = desc_in_editor()?;
                };

                return tau.add(task).await
            }

            TauSubcommand::Modify { values } => {
                if args.filters.is_empty() {
                    no_filter_warn()
                }
                let base_task = task_from_cli(values)?;
                for task in tasks {
                    tau.update(task.id.into(), base_task.clone()).await?;
                }
                Ok(())
            }

            TauSubcommand::Start => {
                if args.filters.is_empty() {
                    no_filter_warn()
                }
                let state = State::Start;
                for task in tasks {
                    tau.set_state(task.id.into(), &state).await?;
                }
                Ok(())
            }

            TauSubcommand::Open => {
                if args.filters.is_empty() {
                    no_filter_warn()
                }
                let state = State::Open;
                for task in tasks {
                    tau.set_state(task.id.into(), &state).await?;
                }
                Ok(())
            }

            TauSubcommand::Pause => {
                if args.filters.is_empty() {
                    no_filter_warn()
                }
                let state = State::Pause;
                for task in tasks {
                    tau.set_state(task.id.into(), &state).await?;
                }
                Ok(())
            }

            TauSubcommand::Stop => {
                if args.filters.is_empty() {
                    no_filter_warn()
                }
                let state = State::Stop;
                for task in tasks {
                    tau.set_state(task.id.into(), &state).await?;
                }
                Ok(())
            }

            TauSubcommand::Comment { content } => {
                if args.filters.is_empty() {
                    no_filter_warn()
                }
                for task in tasks {
                    if content.is_empty() {
                        let task = tau.get_task_by_id(task.id.into()).await?;
                        let comments = comments_as_string(task.comments);
                        println!("Comments {}:\n{}", task.id, comments);
                    } else {
                        tau.set_comment(task.id.into(), &content.join(" ")).await?;
                    }
                }
                Ok(())
            }

            TauSubcommand::Info => {
                for task in tasks {
                    let task = tau.get_task_by_id(task.id.into()).await?;
                    print_task_info(task)?;
                }
                Ok(())
            }
            TauSubcommand::Switch { workspace } => tau.switch_ws(workspace).await,

            TauSubcommand::Export { path } => {
                let path = path.unwrap_or_else(|| DEFAULT_PATH.into());
                let res = tau.export_to(path.clone()).await?;

                if res {
                    info!("Exported to {}", path);
                } else {
                    error!("Error exporting to {}", path);
                }

                Ok(())
            }

            TauSubcommand::Import { path } => {
                let path = path.unwrap_or_else(|| DEFAULT_PATH.into());
                let res = tau.import_from(path.clone()).await?;

                if res {
                    info!("Imported from {}", path);
                } else {
                    error!("Error importing from {}", path);
                }

                Ok(())
            }

            TauSubcommand::Log { month, assignee } => {
                match month {
                    Some(date) => {
                        let ts = to_naivedate(date.clone())?.and_hms(12, 0, 0).timestamp();
                        let tasks = tau.get_stop_tasks(Some(ts)).await?;
                        drawdown(date, tasks, assignee)?;
                    }
                    None => {
                        let ws = tau.get_ws().await?;
                        let tasks = tau.get_stop_tasks(None).await?;
                        print_task_list(tasks, ws)?;
                    }
                }

                Ok(())
            }

            TauSubcommand::List => {
                let ws = tau.get_ws().await?;
                print_task_list(tasks, ws)
            }
        },
        None => {
            let ws = tau.get_ws().await?;
            print_task_list(tasks, ws)
        }
    }?;

    tau.close_connection().await
}
