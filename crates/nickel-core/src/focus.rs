#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FocusTransaction(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusRequest<Surface> {
    pub transaction: FocusTransaction,
    pub surface: Surface,
}

#[derive(Debug)]
pub struct FocusTransactions<Surface> {
    next: u64,
    requested: Option<FocusRequest<Surface>>,
    acknowledged: Option<FocusRequest<Surface>>,
}

impl<Surface> Default for FocusTransactions<Surface> {
    fn default() -> Self {
        Self {
            next: 0,
            requested: None,
            acknowledged: None,
        }
    }
}

impl<Surface: Clone + Eq> FocusTransactions<Surface> {
    pub fn request(&mut self, surface: Surface) -> FocusRequest<Surface> {
        self.next = self.next.wrapping_add(1).max(1);
        let request = FocusRequest {
            transaction: FocusTransaction(self.next),
            surface,
        };
        self.requested = Some(request.clone());
        request
    }

    pub fn acknowledge(&mut self, request: &FocusRequest<Surface>) -> bool {
        if self.requested.as_ref() != Some(request) {
            return false;
        }
        self.acknowledged = Some(request.clone());
        true
    }

    pub fn loses_current(&mut self, request: &FocusRequest<Surface>) -> bool {
        if self.acknowledged.as_ref() != Some(request) {
            return false;
        }
        self.acknowledged = None;
        true
    }

    pub fn requested(&self) -> Option<&FocusRequest<Surface>> {
        self.requested.as_ref()
    }

    pub fn acknowledged(&self) -> Option<&FocusRequest<Surface>> {
        self.acknowledged.as_ref()
    }
}
