use synos_status::{IntoStatus, Status};
use synos_synfs::SynFs;

use crate::{BuildRequest, BuildResult, CompilerService, Error, JobId, NetworkPolicy, Target};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelfHostStage {
    Runtime,
    Compiler,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelfHostError {
    Compiler(Error),
    InvalidRequest,
    InvalidTransition,
    TargetMismatch,
    InvalidArtifact,
}

impl From<Error> for SelfHostError {
    fn from(error: Error) -> Self {
        Self::Compiler(error)
    }
}

impl IntoStatus for SelfHostError {
    fn status(self) -> Status {
        match self {
            Self::Compiler(error) => error.status(),
            Self::InvalidRequest | Self::InvalidTransition => Status::INVALID_ARGUMENT,
            Self::TargetMismatch | Self::InvalidArtifact => Status::CORRUPT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelfHostRequest {
    pub runtime: BuildRequest,
    pub compiler: BuildRequest,
}

impl SelfHostRequest {
    pub fn new(runtime: BuildRequest, compiler: BuildRequest) -> Result<Self, SelfHostError> {
        runtime.validate()?;
        compiler.validate()?;
        if runtime.target != compiler.target {
            return Err(SelfHostError::TargetMismatch);
        }
        if runtime.locked
            && compiler.locked
            && runtime.network == NetworkPolicy::Denied
            && compiler.network == NetworkPolicy::Denied
        {
            return Ok(Self { runtime, compiler });
        }
        Err(SelfHostError::InvalidRequest)
    }

    pub const fn request(&self, stage: SelfHostStage) -> BuildRequest {
        match stage {
            SelfHostStage::Runtime => self.runtime,
            SelfHostStage::Compiler => self.compiler,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelfHostResult {
    pub runtime: BuildResult,
    pub compiler: BuildResult,
}

pub struct SelfHostSession {
    request: SelfHostRequest,
    runtime_job: Option<JobId>,
    compiler_job: Option<JobId>,
    runtime_result: Option<BuildResult>,
    compiler_result: Option<BuildResult>,
}

impl SelfHostSession {
    pub fn new(request: SelfHostRequest) -> Self {
        Self {
            request,
            runtime_job: None,
            compiler_job: None,
            runtime_result: None,
            compiler_result: None,
        }
    }

    pub const fn request(&self) -> SelfHostRequest {
        self.request
    }

    pub fn submit_runtime<const JOBS: usize, const CACHE: usize, const BLOCKS: usize>(
        &mut self,
        service: &mut CompilerService<JOBS, CACHE>,
        filesystem: &SynFs<BLOCKS>,
    ) -> Result<JobId, SelfHostError> {
        if self.runtime_job.is_some() || self.runtime_result.is_some() {
            return Err(SelfHostError::InvalidTransition);
        }
        let job = service.submit_from_synfs(filesystem, self.request.runtime)?;
        self.runtime_job = Some(job);
        Ok(job)
    }

    pub fn start_runtime<const JOBS: usize, const CACHE: usize>(
        &self,
        service: &mut CompilerService<JOBS, CACHE>,
        now_us: u64,
    ) -> Result<(), SelfHostError> {
        let job = self.runtime_job.ok_or(SelfHostError::InvalidTransition)?;
        service.start_at(job, now_us)?;
        Ok(())
    }

    pub fn complete_runtime<const JOBS: usize, const CACHE: usize>(
        &mut self,
        service: &mut CompilerService<JOBS, CACHE>,
        result: BuildResult,
    ) -> Result<(), SelfHostError> {
        let job = self.runtime_job.ok_or(SelfHostError::InvalidTransition)?;
        validate_artifact(result, self.request.runtime.target)?;
        service.complete(job, result)?;
        self.runtime_result = Some(result);
        Ok(())
    }

    pub fn submit_compiler<const JOBS: usize, const CACHE: usize, const BLOCKS: usize>(
        &mut self,
        service: &mut CompilerService<JOBS, CACHE>,
        filesystem: &SynFs<BLOCKS>,
    ) -> Result<JobId, SelfHostError> {
        if self.runtime_result.is_none()
            || self.compiler_job.is_some()
            || self.compiler_result.is_some()
        {
            return Err(SelfHostError::InvalidTransition);
        }
        let job = service.submit_from_synfs(filesystem, self.request.compiler)?;
        self.compiler_job = Some(job);
        Ok(job)
    }

    pub fn start_compiler<const JOBS: usize, const CACHE: usize>(
        &self,
        service: &mut CompilerService<JOBS, CACHE>,
        now_us: u64,
    ) -> Result<(), SelfHostError> {
        let job = self.compiler_job.ok_or(SelfHostError::InvalidTransition)?;
        service.start_at(job, now_us)?;
        Ok(())
    }

    pub fn complete_compiler<const JOBS: usize, const CACHE: usize>(
        &mut self,
        service: &mut CompilerService<JOBS, CACHE>,
        result: BuildResult,
    ) -> Result<(), SelfHostError> {
        let job = self.compiler_job.ok_or(SelfHostError::InvalidTransition)?;
        validate_artifact(result, self.request.compiler.target)?;
        service.complete(job, result)?;
        self.compiler_result = Some(result);
        Ok(())
    }

    pub const fn result(&self) -> Option<SelfHostResult> {
        match (self.runtime_result, self.compiler_result) {
            (Some(runtime), Some(compiler)) => Some(SelfHostResult { runtime, compiler }),
            _ => None,
        }
    }
}

fn validate_artifact(result: BuildResult, target: Target) -> Result<(), SelfHostError> {
    if result.target != target || result.package.is_zero() || result.payload.is_zero() {
        return Err(SelfHostError::InvalidArtifact);
    }
    Ok(())
}
