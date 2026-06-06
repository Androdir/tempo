package com.tempo.mobile

import android.app.Application
import android.content.Context
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import java.util.concurrent.TimeUnit

class TempoApp : Application() {
    override fun onCreate() {
        super.onCreate()
        // Keep the periodic job alive across reboots/app updates (KEEP = don't reset
        // the schedule if it already exists).
        scheduleSync(this)
    }

    companion object {
        const val SYNC_WORK = "tempo_sync"

        private fun netConstraints() = Constraints.Builder()
            .setRequiredNetworkType(NetworkType.CONNECTED)
            .build()

        /** Schedule the recurring upload (every 15 min — WorkManager's minimum). */
        fun scheduleSync(ctx: Context) {
            val request = PeriodicWorkRequestBuilder<SyncWorker>(15, TimeUnit.MINUTES)
                .setConstraints(netConstraints())
                .build()
            WorkManager.getInstance(ctx)
                .enqueueUniquePeriodicWork(SYNC_WORK, ExistingPeriodicWorkPolicy.KEEP, request)
        }

        /** Kick a one-off sync now (e.g. right after pairing) so data shows up fast. */
        fun syncNow(ctx: Context) {
            val request = OneTimeWorkRequestBuilder<SyncWorker>()
                .setConstraints(netConstraints())
                .build()
            WorkManager.getInstance(ctx).enqueue(request)
        }
    }
}
