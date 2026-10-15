#ifndef HANDY_MEETING_CAPTURE_BRIDGE_H
#define HANDY_MEETING_CAPTURE_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

// Callback pointers are borrowed only for the duration of the call. Audio is
// mono 16 kHz f32, at most 4096 samples per call. PTS is host-clock uptime ns.
// Track 0 is microphone, 1 is system/application audio. Track 255 with an error
// string and no samples is a recoverable notice; other errors are terminal.
// The callback must copy synchronously and must never wait for a consumer.
typedef void (*HandyMeetingCallback)(void *context, uint8_t track,
    uint64_t timestamp_ns, const float *samples, size_t count, const char *error);

// Blocking entry points: call from a worker thread, never the app's main thread.
// sources may request Screen & System Audio Recording; start may also request Microphone.
// Strings returned through result/error must be freed with handy_meeting_free.
int32_t handy_meeting_sources(char **result, char **error);
void *handy_meeting_start(const char *microphone_id, int32_t application_pid,
    HandyMeetingCallback callback, void *context, char **error);
// Asynchronously schedules the same bounded recovery used for hardware changes.
// Borrows the live handle; serialize this call against stop. Never frees it.
void handy_meeting_refresh(void *handle);
// Consumes the handle even on error. On return, no callback can use context.
int32_t handy_meeting_stop(void *handle, char **error);
uint64_t handy_meeting_host_time_ns(void);
void handy_meeting_free(char *string);

#endif
