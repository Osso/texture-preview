local M = {}

local BIN = "/home/osso/.cargo/bin/texture-preview"

function M:peek(job)
	local cache = ya.file_cache(job)
	if not cache then
		return
	end

	if not fs.cha(cache) then
		local child = Command(BIN)
			:arg({ tostring(job.file.url), tostring(cache) })
			:stdout(Command.NULL)
			:stderr(Command.PIPED)
			:output()

		if not child or not child.status.success then
			local msg = child and child.stderr or "conversion failed"
			ya.preview_widget(job, { ui.Text(tostring(msg)) })
			return
		end
	end

	local _, err = ya.image_show(cache, job.area)
	ya.preview_widget(job, err)
end

function M:preload(job)
	local cache = ya.file_cache(job)
	if not cache or fs.cha(cache) then
		return true
	end

	local child = Command(BIN)
		:arg({ tostring(job.file.url), tostring(cache) })
		:stdout(Command.NULL)
		:stderr(Command.NULL)
		:status()

	return child ~= nil and child.success
end

function M:seek(job)
	require("image"):seek(job)
end

return M
