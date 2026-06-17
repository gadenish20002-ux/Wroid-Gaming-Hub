use wroid_runtime::ContactId;

use crate::MAX_SLOT_COUNT;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Slot {
    pub(crate) contact_id: Option<ContactId>,
    pub(crate) x: u32,
    pub(crate) y: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SlotState {
    pub(crate) slots: [Slot; MAX_SLOT_COUNT as usize],
    pub(crate) slot_count: u16,
    pub(crate) active_count: u16,
}

impl SlotState {
    pub(crate) fn new(slot_count: u16) -> Self {
        Self {
            slots: [Slot::default(); MAX_SLOT_COUNT as usize],
            slot_count,
            active_count: 0,
        }
    }

    pub(crate) fn active_slot(&self, contact_id: ContactId) -> Option<usize> {
        self.slots[..usize::from(self.slot_count)]
            .iter()
            .position(|slot| slot.contact_id == Some(contact_id))
    }

    pub(crate) fn free_slot(&self) -> Option<usize> {
        self.slots[..usize::from(self.slot_count)]
            .iter()
            .position(|slot| slot.contact_id.is_none())
    }

    pub(crate) fn primary_position(&self) -> Option<(u32, u32)> {
        self.slots[..usize::from(self.slot_count)]
            .iter()
            .find(|slot| slot.contact_id.is_some())
            .map(|slot| (slot.x, slot.y))
    }
}
