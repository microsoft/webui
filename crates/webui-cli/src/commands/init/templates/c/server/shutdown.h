// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#ifndef WEBUI_STARTER_SHUTDOWN_H
#define WEBUI_STARTER_SHUTDOWN_H

#include <stdio.h>
#include <time.h>
#include <errno.h>
#ifdef _WIN32
#include <windows.h>
#else
#include <pthread.h>
#endif

static unsigned host_active_requests = 0;
static int host_draining = 0;
#ifdef _WIN32
static SRWLOCK host_requests_lock = SRWLOCK_INIT;
static CONDITION_VARIABLE host_requests_idle = CONDITION_VARIABLE_INIT;
static void host_lock_requests(void) { AcquireSRWLockExclusive(&host_requests_lock); }
static void host_unlock_requests(void) { ReleaseSRWLockExclusive(&host_requests_lock); }
static void host_signal_idle(void) { WakeAllConditionVariable(&host_requests_idle); }
#else
static pthread_mutex_t host_requests_lock = PTHREAD_MUTEX_INITIALIZER;
#ifdef __APPLE__
static pthread_cond_t host_requests_idle = PTHREAD_COND_INITIALIZER;
#else
static pthread_cond_t host_requests_idle;
#endif
static void host_lock_requests(void) { pthread_mutex_lock(&host_requests_lock); }
static void host_unlock_requests(void) { pthread_mutex_unlock(&host_requests_lock); }
static void host_signal_idle(void) { pthread_cond_broadcast(&host_requests_idle); }
#endif

static int host_init_requests(void) {
#if !defined(_WIN32) && !defined(__APPLE__)
    pthread_condattr_t attributes;
    if (pthread_condattr_init(&attributes) != 0) return 0;
    int status = pthread_condattr_setclock(&attributes, CLOCK_MONOTONIC);
    if (status == 0) status = pthread_cond_init(&host_requests_idle, &attributes);
    pthread_condattr_destroy(&attributes);
    return status == 0;
#else
    return 1;
#endif
}

static void host_free_requests(void) {
#ifndef _WIN32
    pthread_cond_destroy(&host_requests_idle);
#endif
}

#ifndef _WIN32
static int host_wait_requests(const struct timespec *deadline) {
#ifdef __APPLE__
    struct timespec now, remaining;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return errno;
    remaining.tv_sec = deadline->tv_sec - now.tv_sec;
    remaining.tv_nsec = deadline->tv_nsec - now.tv_nsec;
    if (remaining.tv_nsec < 0) {
        --remaining.tv_sec;
        remaining.tv_nsec += 1000000000;
    }
    if (remaining.tv_sec < 0) return ETIMEDOUT;
    return pthread_cond_timedwait_relative_np(&host_requests_idle, &host_requests_lock, &remaining);
#else
    return pthread_cond_timedwait(&host_requests_idle, &host_requests_lock, deadline);
#endif
}
#endif

static int host_drain_requests(void) {
    host_lock_requests();
    host_draining = 1;
    int success = 1;
#ifdef _WIN32
    ULONGLONG deadline = GetTickCount64() + 10000;
    while (host_active_requests) {
        ULONGLONG now = GetTickCount64();
        if (now >= deadline ||
            !SleepConditionVariableSRW(&host_requests_idle, &host_requests_lock,
                                       (DWORD)(deadline - now), 0)) {
            success = 0;
            break;
        }
    }
#else
    struct timespec deadline;
    if (clock_gettime(CLOCK_MONOTONIC, &deadline) != 0) {
        success = 0;
    } else {
        deadline.tv_sec += 10;
        while (host_active_requests) {
            if (host_wait_requests(&deadline) != 0) {
                success = 0;
                break;
            }
        }
    }
#endif
    host_unlock_requests();
    if (!success) fprintf(stderr, "Cannot finish active HTTP requests before the shutdown deadline\n");
    return success;
}

#endif
