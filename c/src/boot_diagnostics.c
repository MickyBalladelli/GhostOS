#include "ghostos/boot_diagnostics.h"

static const uint8_t diagnostic_magic[8]={'S','Y','N','B','T','D','0','1'};
static ghostos_boot_diagnostic_load store_load;
static ghostos_boot_diagnostic_save store_save;
static void *store_context;

static uint16_t read16(const uint8_t*p){return (uint16_t)p[0]|((uint16_t)p[1]<<8);}
static uint32_t read32(const uint8_t*p){return (uint32_t)p[0]|((uint32_t)p[1]<<8)|((uint32_t)p[2]<<16)|((uint32_t)p[3]<<24);}
static uint64_t read64(const uint8_t*p){uint64_t v=0;for(size_t i=0;i<8;i++)v|=(uint64_t)p[i]<<(i*8);return v;}
static void write16(uint8_t*p,uint16_t v){p[0]=(uint8_t)v;p[1]=(uint8_t)(v>>8);}
static void write32(uint8_t*p,uint32_t v){for(size_t i=0;i<4;i++)p[i]=(uint8_t)(v>>(i*8));}
static void write64(uint8_t*p,uint64_t v){for(size_t i=0;i<8;i++)p[i]=(uint8_t)(v>>(i*8));}
static uint32_t checksum(const uint8_t*p,size_t n){uint32_t h=UINT32_C(0x811c9dc5);for(size_t i=0;i<n;i++){h^=p[i];h*=UINT32_C(16777619);}return h;}
static bool valid_status(uint32_t raw){ghostos_status out;return ghostos_status_from_raw(raw,&out);}
static bool valid_state(uint8_t raw){return raw>=GHOSTOS_BOOT_IN_PROGRESS&&raw<=GHOSTOS_BOOT_SUCCEEDED;}

bool ghostos_boot_stage_from_raw(uint8_t raw,ghostos_boot_stage*out){if(raw<1||raw>8)return false;if(out)*out=(ghostos_boot_stage)raw;return true;}
const char *ghostos_boot_stage_name(ghostos_boot_stage s){switch(s){case GHOSTOS_BOOT_KERNEL_ENTRY:return "kernel-entry";case GHOSTOS_BOOT_INFO_VALIDATED:return "boot-info";case GHOSTOS_BOOT_MEMORY_READY:return "memory";case GHOSTOS_BOOT_ARCHITECTURE_READY:return "architecture";case GHOSTOS_BOOT_HARDWARE_READY:return "hardware";case GHOSTOS_BOOT_STORAGE_READY:return "storage";case GHOSTOS_BOOT_SERVICES_READY:return "services";case GHOSTOS_BOOT_USER_HANDOFF:return "user-handoff";default:return "";}}
ghostos_boot_diagnostics ghostos_boot_diagnostics_initial(uint64_t id){ghostos_boot_attempt current={id?id:1,GHOSTOS_BOOT_KERNEL_ENTRY,GHOSTOS_BOOT_IN_PROGRESS,GHOSTOS_STATUS_PENDING,false};ghostos_boot_attempt empty={0,GHOSTOS_BOOT_KERNEL_ENTRY,GHOSTOS_BOOT_FAILED,0,false};return (ghostos_boot_diagnostics){current,empty,false,0};}

void ghostos_boot_diagnostics_begin(const ghostos_boot_diagnostics*previous,bool has_previous,ghostos_boot_diagnostics*next,ghostos_boot_attempt*reported,bool*has_reported){
    bool report=false;ghostos_boot_attempt failure={0,GHOSTOS_BOOT_KERNEL_ENTRY,GHOSTOS_BOOT_FAILED,0,false};
    if(!has_previous||!previous){*next=ghostos_boot_diagnostics_initial(1);}
    else{
        *next=*previous;
        switch(previous->current.state){
            case GHOSTOS_BOOT_FAILED:next->last_failure=previous->current;next->has_last_failure=true;failure=next->last_failure;report=true;break;
            case GHOSTOS_BOOT_IN_PROGRESS:failure=(ghostos_boot_attempt){previous->current.id,previous->current.stage,GHOSTOS_BOOT_FAILED,GHOSTOS_STATUS_BUSY,true};next->last_failure=failure;next->has_last_failure=true;next->failure_count=previous->failure_count==UINT64_MAX?UINT64_MAX:previous->failure_count+1;report=true;break;
            case GHOSTOS_BOOT_SUCCEEDED:break;
        }
        uint64_t id=previous->current.id==UINT64_MAX?UINT64_MAX:previous->current.id+1;if(id==0)id=1;if(id==previous->current.id)id=1;
        next->current=(ghostos_boot_attempt){id,GHOSTOS_BOOT_KERNEL_ENTRY,GHOSTOS_BOOT_IN_PROGRESS,GHOSTOS_STATUS_PENDING,false};
    }
    if(has_reported)*has_reported=report;if(report&&reported)*reported=failure;
}

void ghostos_boot_diagnostics_checkpoint(ghostos_boot_diagnostics*d,ghostos_boot_stage stage){if(d&&d->current.state==GHOSTOS_BOOT_IN_PROGRESS)d->current.stage=stage;}
void ghostos_boot_diagnostics_fail(ghostos_boot_diagnostics*d,ghostos_status status){if(!d||d->current.state!=GHOSTOS_BOOT_IN_PROGRESS||!valid_status(status))return;d->current.state=GHOSTOS_BOOT_FAILED;d->current.status=status;d->current.interrupted=false;d->last_failure=d->current;d->has_last_failure=true;if(d->failure_count<UINT64_MAX)++d->failure_count;}
void ghostos_boot_diagnostics_complete(ghostos_boot_diagnostics*d){if(d&&d->current.state==GHOSTOS_BOOT_IN_PROGRESS){d->current.state=GHOSTOS_BOOT_SUCCEEDED;d->current.status=GHOSTOS_STATUS_NORMAL;}}

bool ghostos_boot_diagnostics_encode(const ghostos_boot_diagnostics*d,uint8_t*dst,size_t cap,size_t*length){if(!d||!dst||cap<56||!valid_status(d->current.status)||!valid_state((uint8_t)d->current.state)||!ghostos_boot_stage_from_raw((uint8_t)d->current.stage,0))return false;for(size_t i=0;i<56;i++)dst[i]=0;for(size_t i=0;i<8;i++)dst[i]=diagnostic_magic[i];write16(dst+8,1);write16(dst+10,56);write64(dst+12,d->current.id);dst[20]=(uint8_t)d->current.state;dst[21]=(uint8_t)d->current.stage;dst[22]=d->current.interrupted?1:0;write32(dst+24,d->current.status);if(d->has_last_failure){if(!valid_status(d->last_failure.status)||!ghostos_boot_stage_from_raw((uint8_t)d->last_failure.stage,0))return false;write64(dst+28,d->last_failure.id);dst[36]=(uint8_t)d->last_failure.stage;dst[37]=d->last_failure.interrupted?1:0;write32(dst+38,d->last_failure.status);}write64(dst+42,d->failure_count);write32(dst+50,checksum(dst,50));if(length)*length=56;return true;}

bool ghostos_boot_diagnostics_decode(const uint8_t*src,size_t len,ghostos_boot_diagnostics*out){if(!src||!out||len<56)return false;for(size_t i=0;i<8;i++)if(src[i]!=diagnostic_magic[i])return false;if(read16(src+8)!=1||read16(src+10)!=56||read32(src+50)!=checksum(src,50)||src[23]!=0||src[54]!=0||src[55]!=0||!valid_state(src[20]))return false;ghostos_boot_stage stage;if(!ghostos_boot_stage_from_raw(src[21],&stage))return false;uint32_t status=read32(src+24);if(!valid_status(status))return false;ghostos_boot_attempt current={read64(src+12),stage,(ghostos_boot_state)src[20],status,src[22]!=0};if(!current.id)return false;uint64_t failure_id=read64(src+28);ghostos_boot_diagnostics d=ghostos_boot_diagnostics_initial(current.id);d.current=current;d.failure_count=read64(src+42);if(failure_id){ghostos_boot_stage fs;if(!ghostos_boot_stage_from_raw(src[36],&fs)||!valid_status(read32(src+38)))return false;d.last_failure=(ghostos_boot_attempt){failure_id,fs,GHOSTOS_BOOT_FAILED,read32(src+38),src[37]!=0};d.has_last_failure=true;}*out=d;return true;}

void ghostos_boot_diagnostics_set_store(ghostos_boot_diagnostic_load load,ghostos_boot_diagnostic_save save,void*context){store_load=load;store_save=save;store_context=context;}
static bool load_diagnostics(ghostos_boot_diagnostics*out){uint8_t bytes[56];size_t length=0;return store_load&&store_load(store_context,bytes,sizeof(bytes),&length)&&ghostos_boot_diagnostics_decode(bytes,length,out);}
static void save_diagnostics(const ghostos_boot_diagnostics*d){uint8_t bytes[56];size_t length;if(store_save&&ghostos_boot_diagnostics_encode(d,bytes,sizeof(bytes),&length))store_save(store_context,bytes,length);}
bool ghostos_boot_diagnostic_begin(ghostos_boot_attempt*reported){ghostos_boot_diagnostics previous,next;bool has=load_diagnostics(&previous),has_reported=false;ghostos_boot_attempt r;ghostos_boot_diagnostics_begin(&previous,has,&next,&r,&has_reported);save_diagnostics(&next);if(has_reported&&reported)*reported=r;return has_reported;}
static void update(ghostos_boot_stage stage,ghostos_status status,unsigned action){ghostos_boot_diagnostics d;if(!load_diagnostics(&d))return;if(action==0)ghostos_boot_diagnostics_checkpoint(&d,stage);else if(action==1)ghostos_boot_diagnostics_fail(&d,status);else ghostos_boot_diagnostics_complete(&d);save_diagnostics(&d);}
void ghostos_boot_diagnostic_checkpoint(ghostos_boot_stage stage){update(stage,0,0);}
void ghostos_boot_diagnostic_fail(ghostos_status status){update(0,status,1);}
void ghostos_boot_diagnostic_complete(void){update(0,0,2);}
bool ghostos_boot_diagnostic_last_failure(ghostos_boot_attempt*out){ghostos_boot_diagnostics d;if(!out||!load_diagnostics(&d)||!d.has_last_failure)return false;*out=d.last_failure;return true;}
