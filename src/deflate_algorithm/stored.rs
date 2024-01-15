use crate::{
    deflate::{
        flush_pending, read_buf, zng_tr_stored_block, BlockState, DeflateStream, MAX_STORED,
    },
    Flush,
};

pub fn deflate_stored(stream: &mut DeflateStream, flush: Flush) -> BlockState {
    // Smallest worthy block size when not flushing or finishing. By default
    // this is 32K. This can be as small as 507 bytes for memLevel == 1. For
    // large input and output buffers, the stored block size will be larger.
    let min_block = Ord::min(stream.state.pending.capacity() - 5, stream.state.w_size);

    // Copy as many min_block or larger stored blocks directly to next_out as
    // possible. If flushing, copy the remaining available input to next_out as
    // stored blocks, if there is enough space.

    // unsigned len, left, have, last = 0;
    let mut have;
    let mut last = false;
    let mut used = stream.avail_in;
    loop {
        // maximum deflate stored block length
        let mut len = MAX_STORED;

        // number of header bytes
        have = ((stream.state.bi_valid + 42) / 8) as usize;

        // we need room for at least the header
        if stream.avail_out < have as u32 {
            break;
        }

        let left = stream.state.strstart as isize - stream.state.block_start;
        let left = Ord::max(0, left) as usize;

        have = stream.avail_out as usize - have;

        if len > left + stream.avail_in as usize {
            // limit len to the input
            len = left + stream.avail_in as usize;
        }

        len = Ord::min(len, have);

        // If the stored block would be less than min_block in length, or if
        // unable to copy all of the available input when flushing, then try
        // copying to the window and the pending buffer instead. Also don't
        // write an empty block when flushing -- deflate() does that.
        if len < min_block
            && ((len == 0 && flush != Flush::Finish)
                || flush == Flush::NoFlush
                || len != left + stream.avail_in as usize)
        {
            break;
        }

        // Make a dummy stored block in pending to get the header bytes,
        // including any pending bits. This also updates the debugging counts.
        last = flush == Flush::Finish && len == left + stream.avail_in as usize;
        zng_tr_stored_block(stream.state, &[], last);

        /* Replace the lengths in the dummy stored block with len. */
        stream.state.pending.rewind(4);
        stream.state.pending.extend(&(len as u16).to_le_bytes());
        stream.state.pending.extend(&(!len as u16).to_le_bytes());

        // Write the stored block header bytes.
        flush_pending(stream);

        // TODO debug counts?

        if left > 0 {
            let left = Ord::min(left, len);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    stream.state.window.offset(stream.state.block_start),
                    stream.next_out,
                    left,
                );
            }

            stream.next_out = stream.next_out.wrapping_add(left);
            stream.avail_out = stream.avail_out.wrapping_sub(left as _);
            stream.total_out = stream.total_out.wrapping_add(left as _);
            stream.state.block_start += left as isize;
            len -= left;
        }

        // Copy uncompressed bytes directly from next_in to next_out, updating the check value.
        if len > 0 {
            read_buf(stream, stream.next_out, len);
            stream.next_out = stream.next_out.wrapping_add(len as _);
            stream.avail_out = stream.avail_out.wrapping_sub(len as _);
            stream.total_out = stream.total_out.wrapping_add(len as _);
        }

        if last {
            break;
        }
    }

    // Update the sliding window with the last s->w_size bytes of the copied
    // data, or append all of the copied data to the existing window if less
    // than s->w_size bytes were copied. Also update the number of bytes to
    // insert in the hash tables, in the event that deflateParams() switches to
    // a non-zero compression level.
    used -= stream.avail_in; /* number of input bytes directly copied */

    if used > 0 {
        let state = &mut stream.state;
        // If any input was used, then no unused input remains in the window, therefore s->block_start == s->strstart.
        if used as usize >= state.w_size {
            /* supplant the previous history */
            state.matches = 2; /* clear hash */

            unsafe {
                std::ptr::copy_nonoverlapping(
                    stream.next_in.wrapping_sub(state.w_size),
                    state.window,
                    state.w_size,
                );
            }

            state.strstart = state.w_size;
            state.insert = state.strstart;
        } else {
            if state.window_size - state.strstart <= used as usize {
                /* Slide the window down. */
                state.strstart -= state.w_size;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        state.window.wrapping_add(state.w_size),
                        state.window,
                        state.strstart,
                    );
                }
                if state.matches < 2 {
                    state.matches += 1; /* add a pending slide_hash() */
                }
                state.insert = Ord::min(state.insert, state.strstart);
            }
            unsafe {
                std::ptr::copy_nonoverlapping(
                    stream.next_in.wrapping_sub(used as usize),
                    state.window.wrapping_add(state.strstart),
                    used as usize,
                );
            }

            state.strstart += used as usize;
            state.insert += Ord::min(used as usize, state.w_size - state.insert);
        }
        state.block_start = state.strstart as isize;
    }

    stream.state.high_water = Ord::max(stream.state.high_water, stream.state.strstart);

    if last {
        return BlockState::FinishDone;
    }

    // If flushing and all input has been consumed, then done.
    if flush != Flush::NoFlush
        && flush != Flush::Finish
        && stream.avail_in == 0
        && stream.state.strstart as isize == stream.state.block_start
    {
        return BlockState::BlockDone;
    }

    let have = stream.state.window_size - stream.state.strstart;
    if stream.avail_in as usize > have && stream.state.block_start >= stream.state.w_size as isize {
        todo!("fill window");
    }

    let have = Ord::min(have, stream.avail_in as usize);
    if have > 0 {
        read_buf(
            stream,
            stream.state.window.wrapping_add(stream.state.strstart),
            have,
        );

        let state = &mut stream.state;
        state.strstart += have;
        state.insert += Ord::min(have, state.w_size - state.insert);
    }

    let state = &mut stream.state;
    state.high_water = Ord::max(state.high_water, state.strstart);

    // There was not enough avail_out to write a complete worthy or flushed
    // stored block to next_out. Write a stored block to pending instead, if we
    // have enough input for a worthy block, or if flushing and there is enough
    // room for the remaining input as a stored block in the pending buffer.

    // number of header bytes
    let have = ((state.bi_valid + 42) >> 3) as usize;

    // maximum stored block length that will fit in pending:
    let have = Ord::min(state.pending.capacity() - have, MAX_STORED);
    let min_block = Ord::min(have, state.w_size);
    let left = state.strstart as isize - state.block_start;

    if left >= min_block as isize
        || ((left > 0 || flush == Flush::Finish)
            && flush != Flush::NoFlush
            && stream.avail_in == 0
            && left <= have as isize)
    {
        let len = Ord::min(left as usize, have); // TODO wrapping?
        last = flush == Flush::Finish && stream.avail_in == 0 && len == (left as usize);

        {
            // TODO hack remove
            let mut tmp = vec![0; len];

            unsafe {
                std::ptr::copy_nonoverlapping(
                    state.window.offset(state.block_start),
                    tmp.as_mut_ptr(),
                    len,
                )
            }

            zng_tr_stored_block(state, &tmp, last);
        }

        state.block_start += len as isize;
        flush_pending(stream);
    }

    // We've done all we can with the available input and output.
    if last {
        BlockState::FinishStarted
    } else {
        BlockState::NeedMore
    }
}