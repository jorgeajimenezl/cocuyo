use std::collections::HashSet;

use iced::Task;

use crate::ambient::BulbInfo;

#[derive(Debug, Clone)]
pub enum BulbSetupMessage {
    Scan,
    BulbsDiscovered(Vec<BulbInfo>),
    ToggleBulb(String),
    Done,
}

pub struct BulbSetupState {
    discovered_bulbs: Vec<BulbInfo>,
    selected_bulbs: HashSet<String>,
    is_scanning: bool,
}

impl BulbSetupState {
    pub fn new() -> Self {
        Self {
            discovered_bulbs: Vec::new(),
            selected_bulbs: HashSet::new(),
            is_scanning: false,
        }
    }

    pub fn update(&mut self, msg: BulbSetupMessage) -> Task<BulbSetupMessage> {
        match msg {
            BulbSetupMessage::Scan => {
                self.is_scanning = true;
                self.discovered_bulbs.clear();
                Task::perform(
                    crate::ambient::discover_bulbs(),
                    BulbSetupMessage::BulbsDiscovered,
                )
            }
            BulbSetupMessage::BulbsDiscovered(bulbs) => {
                self.is_scanning = false;
                self.discovered_bulbs = bulbs;
                Task::none()
            }
            BulbSetupMessage::ToggleBulb(mac) => {
                if !self.selected_bulbs.remove(&mac) {
                    self.selected_bulbs.insert(mac);
                }
                Task::none()
            }
            BulbSetupMessage::Done => Task::none(),
        }
    }

    pub fn discovered_bulbs(&self) -> &[BulbInfo] {
        &self.discovered_bulbs
    }

    pub fn selected_bulbs(&self) -> &HashSet<String> {
        &self.selected_bulbs
    }

    pub fn is_scanning(&self) -> bool {
        self.is_scanning
    }

    pub fn has_selected_bulbs(&self) -> bool {
        !self.selected_bulbs.is_empty()
    }

    pub fn selected_bulbs_vec(&self) -> Vec<String> {
        self.selected_bulbs.iter().cloned().collect()
    }

    pub fn selected_bulb_infos(&self) -> Vec<BulbInfo> {
        self.discovered_bulbs
            .iter()
            .filter(|b| self.selected_bulbs.contains(&b.mac))
            .cloned()
            .collect()
    }
}
