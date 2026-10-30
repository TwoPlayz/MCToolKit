PlayerInfo = {
    styler = {
        type = EXTENSION_TYPE.NBT_EDITOR_STYLE,
        recursive = false
    }
}

-- CUSTOM FUNCTIONS

function PlayerInfo.styler:ticksToTime(ticks)
    local timeString = ""

    local secs = ticks/20
    local mins = (math.floor(secs))//60
    local hours = mins//60
    local days = hours//24

    if(days > 0) then timeString = timeString .. tostring(days) .. " days, " end
    if(hours > 0) then timeString = timeString .. tostring(hours%24) .. " hours, " end
    if(mins > 0) then timeString = timeString .. tostring(mins%60) .. " minutes, " end
    if(secs > 0) then timeString = timeString .. string.gsub(string.format("%.1f", math.abs(secs%60)), "%.0", "") .. " seconds" end

    return timeString
end

-- SETUP

function PlayerInfo.styler:main(root, context)
    if((context.type & FILE_TYPE.PLAYER) ~= 0 or (context.type & FILE_TYPE.LEVEL) ~= 0) then
        if(context.edition == EDITION.JAVA) then
            self:Java(root, context)
        elseif(context.edition == EDITION.BEDROCK) then
            self:Bedrock(root)
        elseif(context.edition == EDITION.CONSOLE) then
            self:Console(root)
        end
    end
end

-- MAIN

function PlayerInfo.styler:Java(root, context)
    local player = root

    -- level.dat check
    if(root:contains("Data", TYPE.COMPOUND)) then
        local dataTag = root.lastFound
        if(dataTag:contains("Player", TYPE.COMPOUND)) then player = dataTag.lastFound end
    end

    if(player:contains("playerGameType", TYPE.INT)) then
        local gamemode = player.lastFound

        if(gamemode.value == 0) then
            gamemode.string = "Survival"
            gamemode.color = "lime"
        elseif(gamemode.value == 1) then
            gamemode.string = "Creative"
            gamemode.color = "gold"
        elseif(gamemode.value == 2) then
            gamemode.string = "Adventure"
            gamemode.color = "Cyan"
        elseif(gamemode.value == 3) then
            gamemode.string = "Spectator"
            gamemode.color = "white"
        end

        if(gamemode.string ~= nil) then
            Style:setLabel(gamemode, gamemode.string)
            Style:setLabelColor(gamemode, gamemode.color)
        end
    end

    if(player:contains("previousPlayerGameType", TYPE.INT)) then
        local gamemode = player.lastFound

        if(gamemode.value == 0) then
            gamemode.string = "Survival"
            gamemode.color = "lime"
        elseif(gamemode.value == 1) then
            gamemode.string = "Creative"
            gamemode.color = "gold"
        elseif(gamemode.value == 2) then
            gamemode.string = "Adventure"
            gamemode.color = "Cyan"
        elseif(gamemode.value == 3) then
            gamemode.string = "Spectator"
            gamemode.color = "white"
        end

        if(gamemode.string ~= nil) then
            Style:setLabel(gamemode, gamemode.string)
            Style:setLabelColor(gamemode, gamemode.color)
        end
    end

    if(player:contains("XpLevel", TYPE.INT)) then
        Style:setLabel(player.lastFound, "Level shown on the XP bar")
    end

    if(player:contains("XpP", TYPE.FLOAT)) then
        local percent = player.lastFound

        Style:setLabel(percent, tostring(math.floor(percent.value*100)) .. "% to next XP level")
    end

    if(player:contains("XpTotal", TYPE.INT)) then
        Style:setLabel(player.lastFound, "Total lifetime XP. Usually the same as Score")
    end

    if(player:contains("Score", TYPE.INT)) then
        Style:setLabel(player.lastFound, "Score shown on death")
    end

    if(player:contains("foodLevel", TYPE.INT)) then
        local foodLevel = player.lastFound
        local text = string.gsub(string.format("%.1f", foodLevel.value/2), "%.0", "") .. " Food"

        if(foodLevel.value == 0) then text = text .. " (Starving)"
        elseif(foodLevel.value == 10) then text = text .. " (Half)"
        elseif(foodLevel.value == 20) then text = text .. " (Full)"
        end

        Style:setLabel(foodLevel, text)
        Style:setIcon(foodLevel, "PlayerInfo/images/hunger.png")
    end

    if(player:contains("LastDeathLocation", TYPE.COMPOUND)) then
        local lastDeath = player.lastFound
        local text = ""

        if(lastDeath:contains("pos", TYPE.INT_ARRAY)) then
            local pos = lastDeath.lastFound

            if(pos:getSize() == 12) then
                text = "X:" .. tostring(pos:getInt(0)) .. ", Y:" .. tostring(pos:getInt(4)) .. ", Z:" .. tostring(pos:getInt(8))

                if(lastDeath:contains("dimension", TYPE.STRING)) then
                    local dim = lastDeath.lastFound.value
                    
                    if(dim == "minecraft:overworld") then text = text .. " in the Overworld"
                    elseif(dim == "minecraft:nether") then text = text .. " in the Nether"
                    elseif(dim == "minecraft:end") then text = text .. " in the End"
                    end
                end
            end
        end

        if(text ~= "") then
            Style:setLabel(lastDeath, text)
            Style:setLabelColor(lastDeath, "#bfbfbf")
        end
    end

    if(player:contains("SleepTimer", TYPE.SHORT)) then
        local timer = player.lastFound
        local text = ""

        if(timer.value == 0) then text = "Not in bed"
        elseif(timer.value > 0 and timer.value <= 100) then text = "In bed"
        elseif(timer.value > 100 and timer.value <= 109) then text = "Just woke up"
        end

        if(text ~= "") then
            Style:setLabel(timer, text)
        end

    end

    if(player:contains("FallDistance", TYPE.FLOAT)) then
        local fall = player.lastFound
        local text = ""

        if(fall.value == 0) then text = "Blocks fallen"
        elseif(fall.value > 0) then text = math.floor(fall.value) .. " blocks fallen"
        end

        if(fall.value > 3) then
            text = text .. " (" .. string.gsub(string.format("%.1f", (fall.value-3)/2), "%.0", "") .. " hearts of damage)"
        end

        Style:setLabel(player.lastFound, text)
    end
end

function PlayerInfo.styler:Bedrock(root, context)
    local player = root

    if(player:contains("PlayerGameMode", TYPE.INT)) then
        local gamemode = player.lastFound

        -- 3 & 4 = survival with fly and god mode?

        if(gamemode.value == 0) then
            gamemode.string = "Survival"
            gamemode.color = "lime"
        elseif(gamemode.value == 1) then
            gamemode.string = "Creative"
            gamemode.color = "gold"
        elseif(gamemode.value == 2) then
            gamemode.string = "Adventure"
            gamemode.color = "Cyan"
        elseif(gamemode.value == 5) then
            gamemode.string = "Using world default"
            gamemode.color = "white"
        elseif(gamemode.value == 6) then
            gamemode.string = "Spectator"
            gamemode.color = "white"
        end

        if(gamemode.string ~= nil) then
            Style:setLabel(gamemode, gamemode.string)
            Style:setLabelColor(gamemode, gamemode.color)
        end
    end

    if(player:contains("SleepTimer", TYPE.SHORT)) then
        local timer = player.lastFound
        local text = ""

        if(timer.value == 0) then text = "Not in bed"
        elseif(timer.value > 0 and timer.value <= 100) then text = "In bed"
        elseif(timer.value > 100 and timer.value <= 109) then text = "Just woke up"
        end

        if(text ~= "") then
            Style:setLabel(timer, text)
        end

    end

    if(player:contains("FallDistance", TYPE.FLOAT)) then
        local fall = player.lastFound
        local text = ""

        if(fall.value == 0) then text = "Blocks fallen"
        elseif(fall.value > 0) then text = math.floor(fall.value) .. " blocks fallen"
        end

        if(fall.value > 3) then
            text = text .. " (" .. string.gsub(string.format("%.1f", (fall.value-3)/2), "%.0", "") .. " hearts of damage)"
        end

        Style:setLabel(player.lastFound, text)
    end

    if(player:contains("DeathDimension", TYPE.INT)) then
        local dim = player.lastFound
        local text = ""

        if(dim.value == 0) then text = "Overworld"
        elseif(dim.value == 1) then text = "Nether"
        elseif(dim.value == 2) then text = "The End"
        end

        if(text ~= "")then 
            Style:setLabel(dim, text)
        end
    end

    if(player:contains("DimensionId", TYPE.INT)) then
        local dim = player.lastFound

        if(dim.value == 0) then
            dim.string = "Overworld"
            dim.color = "lime"
        elseif(dim.value == 1) then
            dim.string = "Nether"
            dim.color = "red"
        elseif(dim.value == 2) then
            dim.string = "The End"
            dim.color = "magenta"
        end

        if(dim.string ~= nil) then
            Style:setLabel(dim, dim.string)
            Style:setLabelColor(dim, dim.color)
        end
    end

    if(player:contains("SpawnDimension", TYPE.INT)) then
        local dim = player.lastFound
        local text = ""

        if(dim.value == 0) then text = "Overworld"
        elseif(dim.value == 1) then text = "Nether"
        elseif(dim.value == 2) then text = "The End"
        elseif(dim.value == 3) then text = "Overworld" -- could mean world default.
        end

        if(text ~= "")then 
            Style:setLabel(dim, text)
        end
    end

    if(player:contains("PlayerLevel", TYPE.INT)) then
        Style:setLabel(player.lastFound, "Level shown on the XP bar")
    end

    if(player:contains("PlayerLevelProgress", TYPE.FLOAT)) then
        local percent = player.lastFound

        Style:setLabel(percent, tostring(math.floor(percent.value*100)) .. "% to next XP level")
    end

    if(player:contains("TimeSinceRest", TYPE.INT)) then
        Style:setLabel(player.lastFound, self:ticksToTime(player.lastFound.value))
    end

    if(player:contains("Attributes", TYPE.LIST, TYPE.COMPOUND)) then
        local list = player.lastFound

        for i=0, list.childCount-1 do
            local attr = list:child(i)
            local text = ""

            if(attr:contains("Name", TYPE.STRING)) then
                attr.id = attr.lastFound.value

                if(attr.id == "minecraft:player.hunger") then text = "Hunger"
                elseif(attr.id == "minecraft:player.exhaustion") then text = "Exhaustion"
                elseif(attr.id == "minecraft:player.saturation") then text = "Saturation"
                elseif(attr.id == "minecraft:player.level") then text = "Level"
                elseif(attr.id == "minecraft:player.experience") then text = "Experience"
                end
            end

            if(text ~= "") then
                Style:setLabel(attr, text)
                Style:setLabelColor(attr, "#bfbfbf")
            end
        end
    end
end

function PlayerInfo.styler:Console(root, context)
    local player = root

    if(player:contains("XpLevel", TYPE.INT)) then
        Style:setLabel(player.lastFound, "The level shown on the XP bar")
    end

    if(player:contains("XpP", TYPE.FLOAT)) then
        local percent = player.lastFound

        Style:setLabel(percent, tostring(math.floor(percent.value*100)) .. "% to next XP level")
    end

    if(player:contains("XpTotal", TYPE.INT)) then
        Style:setLabel(player.lastFound, "Total lifetime XP. Usually the same as Score")
    end

    if(player:contains("Score", TYPE.INT)) then
        Style:setLabel(player.lastFound, "Score shown on death")
    end

    if(player:contains("Dimension", TYPE.INT)) then
        local dim = player.lastFound

        if(dim.value == 0) then
            dim.string = "Overworld"
            dim.color = "lime"
        elseif(dim.value == 1) then
            dim.string = "Nether"
            dim.color = "red"
        elseif(dim.value == 2) then
            dim.string = "The End"
            dim.color = "magenta"
        end

        if(dim.string ~= nil) then
            Style:setLabel(dim, dim.string)
            Style:setLabelColor(dim, dim.color)
        end
    end

    if(player:contains("SleepTimer", TYPE.SHORT)) then
        local timer = player.lastFound
        local text = ""

        if(timer.value == 0) then text = "Not in bed"
        elseif(timer.value > 0 and timer.value <= 100) then text = "In bed"
        elseif(timer.value > 100 and timer.value <= 109) then text = "Just woke up"
        end

        if(text ~= "") then
            Style:setLabel(timer, text)
        end

    end

    if(player:contains("FallDistance", TYPE.FLOAT)) then
        local fall = player.lastFound
        local text = ""

        if(fall.value == 0) then text = "Blocks fallen"
        elseif(fall.value > 0) then text = math.floor(fall.value) .. " blocks fallen"
        end

        if(fall.value > 3) then
            text = text .. " (" .. string.gsub(string.format("%.1f", (fall.value-3)/2), "%.0", "") .. " hearts of damage)"
        end

        Style:setLabel(player.lastFound, text)
    end

    if(player:contains("foodLevel", TYPE.INT)) then
        local foodLevel = player.lastFound
        local text = string.gsub(string.format("%.1f", foodLevel.value/2), "%.0", "") .. " Food"

        if(foodLevel.value == 0) then text = text .. " (Starving)"
        elseif(foodLevel.value == 10) then text = text .. " (Half)"
        elseif(foodLevel.value == 20) then text = text .. " (Full)"
        end

        Style:setLabel(foodLevel, text)
        Style:setIcon(foodLevel, "PlayerInfo/images/hunger.png")
    end
end

return PlayerInfo